use super::*;
use pretty_assertions::assert_eq;

/// Lex a command line as the classifier does and ask the arithmetic check.
fn runs_unseen(line: &str) -> bool {
    runs_unseen_in("bash", line)
}

fn runs_unseen_in(shell: &str, line: &str) -> bool {
    let argv = vec![shell.to_string(), "-c".to_string(), line.to_string()];
    let mut arithmetic = Arithmetic::new(&argv);
    let variables = HashMap::new();
    for simple in &shell::simple_commands(&argv).commands {
        let (words, _) = super::super::substitute(simple, &variables);
        arithmetic.note(simple, &words);
    }
    arithmetic.runs_unseen(&argv)
}

fn region_texts(text: &str) -> Vec<&str> {
    regions(text)
        .into_iter()
        .map(|range| &text[range])
        .collect()
}

#[test]
fn regions_cover_every_arithmetic_context() {
    assert_eq!(region_texts("echo $((v + 1))"), vec!["$((v + 1))"]);
    assert_eq!(region_texts("(( v )); :"), vec!["(( v ))"]);
    assert_eq!(region_texts("echo $[v]"), vec!["$[v]"]);
    assert_eq!(region_texts("echo ${a[v]} $a[w]"), vec!["[v]", "[w]"]);
    assert_eq!(region_texts("a[v]=1"), vec!["[v]"]);
    assert_eq!(region_texts("echo ${s:v:w}"), vec![":v:w}"]);
    assert_eq!(
        region_texts("echo ${!v} ${(l:v:)s}"),
        vec!["${!v}", "${(l:v:)s}"]
    );
    assert_eq!(region_texts("[[ v -eq 1 ]]"), vec!["[[ v -eq 1 ]]"]);
    assert_eq!(
        region_texts("[[ -n x && ( $v -gt 1 ) ]]"),
        vec!["[[ -n x && ( $v -gt 1 ) ]]"]
    );
    // Nested arithmetic is one region; parentheses inside do not end it.
    assert_eq!(
        region_texts("x=$(( (a+b) * c[d[e]] )); y"),
        vec!["$(( (a+b) * c[d[e]] ))"]
    );
    // An unclosed region runs to the end.
    assert_eq!(region_texts("echo $(( v"), vec!["$(( v"]);
    // Not arithmetic.
    for text in [
        "echo ${v:-x} ${v:=y} ${v:+z} ${v:?w} ${#v} ${v} $v",
        "[ -n \"$v\" ] && [[ $v == x ]]",
        "grep '[[:digit:]]' f",
        "echo $(cat f)",
    ] {
        assert_eq!(region_texts(text), Vec::<&str>::new(), "{text}");
    }
}

#[test]
fn names_skip_numbers_and_lengths() {
    let found: Vec<&str> = names("v + $w * ${x} - 0x1f + 16#ff + ${#y} + $1 + ${2} + $@").collect();
    assert_eq!(found, vec!["v", "w", "x", "1", "2", "@"]);
}

#[test]
fn numeric_formats() {
    for format in ["%d", "%5.2f\\n", "x %i", "%*s", "%.*s", "%x"] {
        assert!(numeric_format(format), "{format}");
    }
    for format in ["%s\\n", "100%%", "%c", "%b", "plain"] {
        assert!(!numeric_format(format), "{format}");
    }
}

#[test]
fn function_definitions() {
    assert_eq!(
        functions("f() { :; }; function g { :; }; h ( ) { :; }; é() { :; }"),
        vec!["f", "h", "g"]
    );
    assert_eq!(functions("echo $(cat f) ()"), Vec::<String>::new());
}

/// The lexer marks an arithmetic expansion's number apart from command
/// output, but only when bash and zsh certainly read it as arithmetic.
#[test]
fn lexer_tells_arithmetic_from_command_output() {
    let value = |line: &str| {
        let lexed = shell::simple_commands(&[line.to_string()]);
        let assignment = lexed
            .commands
            .iter()
            .find(|command| {
                command
                    .words
                    .first()
                    .is_some_and(|word| word.starts_with("x="))
            })
            .expect("the assignment");
        let value = &assignment.words[0];
        (command_output(value), value.contains(shell::UNSEEN_OUTPUT))
    };
    // Arithmetic: still unseen to every other check, but not command output.
    for line in ["x=$((i + 1))", "x=$((${#a} - 1))", "x=$(($i*2))"] {
        assert_eq!(value(line), (false, true), "{line}");
    }
    // Command output, or not certainly arithmetic.
    for line in [
        "x=$(cat f)",
        "x=`cat f`",
        "x=$((cat f) )",
        "x=$(((1));cat f)",
        "x=$((cat '))';cat f) )",
        "x=$((${y:-))};cat f) )",
        "x=$(( (a+b) / 2 ))",
        "x=$(( $(cat f) ))",
        "x=$(cat f)$((i + 1))",
    ] {
        assert_eq!(value(line), (true, true), "{line}");
    }
}

#[test]
fn filled_variables_in_arithmetic_run_unseen() {
    for line in [
        "v=$(cat notes); echo $((v))",
        "v=$(cat notes); echo $(( $v ))",
        "v=$(cat notes); let v",
        "v=$(cat notes); let \"x=v+1\"",
        "v=$(cat notes); (( v ))",
        "v=$(cat notes); echo $[v]",
        "v=$(cat notes); s=x; echo ${s:v}",
        "v=$(cat notes); a=(1); echo ${a[v]}",
        "v=$(cat notes); a[v]=1",
        "v=$(cat notes); echo ${!v}",
        "v=$(cat notes); [[ v -eq 1 ]]",
        "v=$(cat notes); [[ -n x && $v -gt 1 ]]",
        "v=$(cat notes); declare -i n; n=v",
        "v=$(cat notes); declare -i n=v",
        "declare -i n; n=$(cat notes)",
        "declare -i n; read n < notes",
        "v=$(cat notes); declare \"$v\"=1",
        "v=$(cat notes); read \"$v\" <<< 1",
        "v=$(cat notes); unset \"$v\"",
        "v=$(cat notes); x='a[v]'; declare \"$x\"=1",
        "v=$(cat notes); r=v; echo $((r))",
        "r=v; v=$(cat notes); echo $((r))",
        "v=$(cat notes); for ((i=v; i<0; i++)); do :; done",
        "v=$(cat notes); set -- \"$v\"; echo $(( $1 ))",
        "f() { echo $(( $1 )); }; v=$(cat notes); f \"$v\"",
        "read v < notes; echo $((v))",
        "while read -r v; do echo $((v)); done < notes",
        "mapfile -t lines < notes; echo $(( lines[0] ))",
        "for v in $(cat notes); do echo $((v)); done",
        "echo $(( $(cat notes) + 1 ))",
        "echo $(( a[`cat notes`] ))",
        "v=$(((1));cat notes); echo $((v))",
        // zsh and ksh.
        "v=$(cat notes); echo $a[v]",
        "v=$(cat notes); print -f '%d' \"$v\"",
        "v=$(cat notes); printf '%*s' v x",
        "v=$(cat notes); repeat v; do :; done",
        "v=$(cat notes); exit v",
        "v=$(cat notes); echo ${(l:v:)s}",
        "v=$(cat notes); vared \"$v\"",
        "v=$(cat notes); [[ -v $v ]]",
        "v=$(cat notes); test -v \"$v\"",
        "v=$(cat notes); integer n; n=v",
        "v=$(cat notes); read -t v x",
        "v=$(cat notes); eval let v",
        "v=$(cat notes); noglob let v",
        "v=$(cat notes); printf -v n '%s' \"$v\"; echo $((n))",
        "for a b in $(cat notes); do echo $((b)); done",
        "v=$(cat notes); alias L=let\nL v",
        "v=$(cat notes); alias L='let v'",
    ] {
        assert!(runs_unseen(line), "{line}");
    }
    // ksh's `test` evaluates `-eq` operands; bash's and zsh's do not.
    let line = "v=$(cat notes); [ \"$v\" -eq 1 ]";
    assert!(runs_unseen_in("ksh", line));
    assert!(!runs_unseen_in("zsh", line));
}

#[test]
fn counters_and_plain_uses_still_run() {
    for line in [
        "i=0; ((i++)); echo $((i+1))",
        "i=0; while [ $i -lt 3 ]; do i=$((i+1)); done",
        "for ((i=0; i<10; i++)); do echo $i; done",
        "n=$((2*3)); echo $((n+1)); a[n]=1",
        "start=$(date +%s); echo $(( $(date +%s) - start ))",
        "v=$(cat notes); echo \"$v\" ${v:-x} ${#v} ${v:0:3}",
        "line=$(head -1 notes); echo $(( ${#line} + 1 ))",
        "while read -r line; do printf '%s\\n' \"$line\"; done < notes",
        "count=$(grep -c x notes); [ \"$count\" -gt 0 ] && echo yes",
        "v=$(cat notes); grep '[[:digit:]]' notes",
        "echo $((LINES + COLUMNS))",
    ] {
        assert!(!runs_unseen(line), "{line}");
    }
}

#[test]
fn review_1_regions_and_lists() {
    assert_eq!(
        region_texts("echo ${b[@]:v} ${@:w}"),
        vec!["[@]", ":v}", ":w}"]
    );
    assert_eq!(region_texts("a=([v]=1 x)"), vec!["[v]"]);
    assert_eq!(region_texts("[[ \"]]\" && 1 -eq $v ]]").len(), 1);
    let (found, deep) = lists("a=(x $(cat f)) b+=(y); for x y (1 2) :");
    let names: Vec<Vec<String>> = found.into_iter().map(|list| list.names).collect();
    assert_eq!(
        names,
        vec![
            vec!["a".to_string()],
            vec!["b".to_string()],
            vec!["y".to_string(), "x".to_string()]
        ]
    );
    assert!(!deep);
    assert!(lists(&"a=(".repeat(MAX_LIST_DEPTH + 1)).1);
    assert_eq!(
        defaults("${a:=x}${b=y}${c::=z}${d:-w}"),
        vec!["a", "b", "c"]
    );
}

/// Attacker text cannot make the check super-linear (scripts of up to
/// 256 KiB are classified this way).
#[test]
fn review_1_stays_linear() {
    let big = |unit: &str| unit.repeat(256 * 1024 / unit.len());
    let lines = [
        format!("for {} in {}; do :; done", big("a ").trim(), big("1 ")),
        format!("printf {} x", big("-v x ")),
        format!("eval(){{ :;}}; {} v", big("eval ")),
        format!("alias eval=let; {} v", big("eval ")),
        format!("echo {}", big("${")),
        format!("echo {}", big("${a[")),
        big("[[ "),
        big("a=("),
        big("x x ("),
    ];
    let started = std::time::Instant::now();
    for line in &lines {
        runs_unseen(line);
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
}
