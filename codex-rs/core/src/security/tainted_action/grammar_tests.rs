use super::*;
use pretty_assertions::assert_eq;

/// Each command's words as (text, data), substituted commands marked `$()`.
fn words(text: &str) -> Vec<Vec<(String, bool)>> {
    read(text)
        .expect("readable")
        .commands
        .into_iter()
        .map(|command| {
            let mut words: Vec<(String, bool)> = command
                .words
                .into_iter()
                .map(|word| (word.text, word.data))
                .collect();
            if command.substituted {
                words.insert(0, ("$()".to_string(), true));
            }
            words
        })
        .collect()
}

fn w(text: &str, data: bool) -> (String, bool) {
    (text.to_string(), data)
}

#[test]
fn quoted_and_plain_words_are_data() {
    assert_eq!(
        words(r#"git commit -m 'a; b $(c)' "d e" f=g:h/i,j"#),
        vec![vec![
            w("git", true),
            w("commit", true),
            w("-m", true),
            w("a; b $(c)", true),
            w("d e", true),
            w("f=g:h/i,j", true),
        ]]
    );
}

#[test]
fn expansions_globs_and_escapes_are_not_data() {
    let line = r#"echo "$x" "a\"b" `id` $'\x41' ~/f *.rs {a,b} =ls a\ b"#;
    let data: Vec<bool> = words(line)[0].iter().map(|(_, data)| *data).collect();
    assert_eq!(
        data,
        [
            true, false, false, false, false, false, false, false, false, false
        ]
    );
}

#[test]
fn parts_with_nothing_between_them_are_one_word() {
    assert_eq!(
        words("cat ~/$a$b/c.toml $\"t\" x a#b"),
        vec![vec![
            w("cat", true),
            w("~/$a$b/c.toml", false),
            w("$t", false),
            w("x", true),
            w("a#b", false),
        ]]
    );
}

#[test]
fn substitutions_are_commands_of_their_own() {
    let line = "a $(b \"$(c)\") <(d) <<EOF | e\n$(f)\nEOF\ng() { h; }; for i in j; do k; done; export l=$(m)";
    let names: Vec<(bool, String)> = read(line)
        .expect("readable")
        .commands
        .into_iter()
        .map(|command| (command.substituted, command.words[0].text.clone()))
        .collect();
    let expected: Vec<(bool, String)> = [
        (false, "a"),
        (true, "b"),
        (true, "c"),
        (true, "d"),
        (false, "e"),
        (true, "f"),
        (false, "h"),
        (false, "k"),
        (false, "export"),
        (true, "m"),
    ]
    .into_iter()
    .map(|(substituted, name)| (substituted, name.to_string()))
    .collect();
    assert_eq!(names, expected);
}

#[test]
fn unreadable_text_is_refused() {
    for (text, refusal) in [
        ("echo $(", Refusal::Syntax),
        ("echo 'a", Refusal::Syntax),
        ("if x; then y", Refusal::Syntax),
        ("echo a\u{1}b", Refusal::Characters),
        ("echo \u{e9}|sh", Refusal::Characters),
        ("echo \u{e9};id", Refusal::Characters),
        ("echo a\\\nb", Refusal::LineContinuation),
        ("done x", Refusal::Words),
        ("coproc id", Refusal::Words),
        ("} x", Refusal::Words),
        ("[v]", Refusal::Words),
        ("--expr=1e x", Refusal::Words),
        ("\\ =git x", Refusal::Words),
        ("print('ok')", Refusal::Syntax),
        ("+.,/()[]", Refusal::Syntax),
        ("echo;; id", Refusal::Syntax),
        ("cat <<<\nx", Refusal::Syntax),
    ] {
        assert_eq!(read(text).err(), Some(refusal), "{text:?}");
    }
    assert_eq!(
        read(&"a".repeat(MAX_BYTES + 1)).err(),
        Some(Refusal::TooLarge)
    );
    // Non-ASCII text is read when nothing a shell may join to it follows.
    assert!(read("echo r\u{e9}sum\u{e9} x").is_ok());
}

#[test]
fn deep_nesting_is_refused_not_walked() {
    let depth = 1000;
    let text = format!("{}x{}", "$(".repeat(depth), ")".repeat(depth));
    assert!(matches!(read(&text).err(), Some(Refusal::TooDeep)));
}

#[test]
fn zsh_differences_are_flagged() {
    for text in [
        "repeat 2 echo x",
        "echo ${(e)x}",
        "echo $x[1] $y",
        "echo $x:h",
        "{ echo }",
        "setopt globsubst",
        "alias l=ls; l",
        "$cmd x",
    ] {
        // Flagged, or not read at all.
        assert!(
            read(text).ok().is_none_or(|reading| reading.zsh_unsure),
            "{text:?}"
        );
    }
    for text in ["echo a b", "git log -- x", "f() { g; }", "echo \"$x\" ${y}"] {
        assert!(
            read(text).is_ok_and(|reading| !reading.zsh_unsure),
            "{text:?}"
        );
    }
}

#[test]
fn parse_work_is_bounded_by_the_budget() {
    // Error recovery on unbalanced text is where a parse could do the most
    // work; it stops at the first error, within the budget.
    for unit in [
        "$((",
        "'\"`",
        "case x in (",
        "${x:-'",
        "a=( ",
        "x | ",
        "f() ",
    ] {
        let text = unit.repeat(MAX_BYTES / unit.len());
        match read(&text) {
            Ok(reading) => assert!(reading.parse_checks <= parse_budget(text.len())),
            Err(refusal) => assert!(
                matches!(
                    refusal,
                    Refusal::Syntax | Refusal::ParseBudget | Refusal::TooDeep
                ),
                "{unit:?}: {refusal:?}"
            ),
        }
    }
}

#[test]
fn here_documents_the_scanner_may_forget_are_refused() {
    let nine = "cat <<a\nx\na\n".repeat(9);
    assert_eq!(read(&nine).err(), Some(Refusal::HereDocuments));
    let long = format!("cat <<{}\nx\n", "d".repeat(MAX_DELIMITER_BYTES + 1));
    assert_eq!(read(&long).err(), Some(Refusal::HereDocuments));
    assert!(read(&"cat <<a\nx\na\n".repeat(8)).is_ok());
    // An extended glob pattern bash refuses by default.
    assert_eq!(
        read("case x in @(a|b)) ;; esac").err(),
        Some(Refusal::Unsupported)
    );
}

#[test]
fn reading_work_is_linear() {
    work::assert_linear("grammar", |scale| {
        let _ = read(&"a $(b 'c' \"$d\") | e; ".repeat(20 * scale));
    });
}
