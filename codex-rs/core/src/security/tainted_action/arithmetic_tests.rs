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
        format!("set {}", big("-A x ")),
        format!("alias s='set {}'; s", big("-A a ")),
    ];
    let started = std::time::Instant::now();
    for line in &lines {
        runs_unseen(line);
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
}

/// Review 2 N1: non-ASCII text before `(` must not split a character.
#[test]
fn review_2_non_ascii_before_parenthesis() {
    for line in [
        "git commit -m \"Fix café (v2)\"",
        "echo \"done ✓ (ok)\"",
        "python3 -c \"print('✓ (x)')\"",
        "x é=(1)",
        "é=(1)",
        &format!("echo \"${{a[{}é]}}\"", "x".repeat(254)),
        &format!("echo \"${{a[{}é]}}\"", "x".repeat(253)),
    ] {
        assert!(!runs_unseen(line), "{line}");
    }
}

/// Review 2 N3: names built by quote removal or expansion.
#[test]
fn review_2_built_names_run_unseen() {
    for (shell, tail) in [
        ("bash", "((va\"\"r))"),
        ("bash", "((va''r))"),
        ("bash", "((va\\r))"),
        ("bash", "a[va\"\"r]=1"),
        ("bash", "echo ${a[1+va\"\"r]}"),
        ("bash", "x=va; echo $(( 1 + ${x}r ))"),
        ("zsh", "x=va; y=${x}r; echo $(( 1 + y ))"),
        ("bash", "x=v; x+=ar; echo $(( 1 + $x ))"),
        ("bash", "x=zvar; echo $(( 1 + ${x#z} ))"),
        ("zsh", "x=VAR; echo $(( 1 + ${(L)x} ))"),
        ("bash", "x=va; let \"1+${x}r\""),
        ("bash", "x=var; echo $(( ${!x} ))"),
        ("bash", "x=va; y=r; echo $(( $x$y ))"),
    ] {
        let line = format!("var=$(cat notes); {tail}");
        assert!(runs_unseen_in(shell, &line), "{shell}: {line}");
        // Nothing tainted: built names alone are fine. (The lexer cannot
        // read zsh `${(L)x}`, which counts as output for the fallback.)
        if !tail.contains("${(") {
            assert!(!runs_unseen_in(shell, tail), "{shell}: {tail}");
        }
    }
    // Since the round-6 fallback, variables the line does not assign (`n`,
    // `h` from the environment) count once it holds command output.
    assert!(runs_unseen(
        "v=$(cat f); echo $(( ${#v} + 1 )) $(( ${n:-0} + 1 )) $(( 0x$h ))"
    ));
    for line in [
        "v=$(cat f); echo $(( ${#v} + 1 )) $(( 0x1f ))",
        "v=$(cat f); n=${#v}; echo $(( n + 1 ))",
    ] {
        assert!(!runs_unseen(line), "{line}");
    }
}

/// Review 2 N4-N7: integer specials, `for x;`, more fill sources, and
/// `printf` output that decodes escapes.
#[test]
fn review_2_specials_fills_and_printf() {
    for (shell, line) in [
        ("zsh", "v=$(cat notes); SECONDS=$v"),
        ("bash", "v=$(cat notes); OPTIND=$v"),
        ("ksh", "v=$(cat notes); LINENO=$v"),
        ("zsh", "read v; KEYTIMEOUT=$v"),
        (
            "bash",
            "set -- \"$(cat notes)\"; for x; do echo $((x)); done",
        ),
        (
            "zsh",
            "f(){ for x; do echo $((x)); done; }; f \"$(cat notes)\"",
        ),
        (
            "bash",
            "set -- \"$(cat notes)\"; for x do echo $((x)); done",
        ),
        ("zsh", "read 'v?prompt' < notes; echo $((v))"),
        (
            "zsh",
            "zmodload zsh/system; sysread < notes; echo $((REPLY))",
        ),
        ("zsh", "v=$(cat notes); zparseopts -A o a:; echo $((o[a]))"),
        (
            "zsh",
            "v=$(cat notes); [[ $v = (#b)(*) ]]; echo $((match[1]))",
        ),
        ("zsh", "v=$(cat notes); : ${v/(#m)*/x}; echo $((MATCH))"),
        ("bash", ": ${b[0]:=$(cat notes)}; echo $((b[0]))"),
        ("bash", "v=$(printf 'a[\\x24\\x28cmd\\x290]'); echo $((v))"),
        ("bash", "v=$(printf '%s%s' a b); echo $((v))"),
        ("bash", "v=$(echo -e '\\x61'); echo $((v))"),
    ] {
        assert!(runs_unseen_in(shell, line), "{shell}: {line}");
    }
    for line in [
        "while getopts ab: opt; do :; done; shift $((OPTIND-1))",
        "OPTIND=1; SECONDS=0; echo $((SECONDS))",
        "for x in 1 2; do echo $((x)); done",
        "v=$(echo 5); echo $((v+1))",
    ] {
        assert!(!runs_unseen(line), "{line}");
    }
}

/// Review 2 should-fix: redirect targets are not operands, and a value
/// that still holds a substitution after decoding runs it.
#[test]
fn review_2_redirects_and_decoded_values() {
    for line in [
        "f=$(mktemp); read -r x < \"$f\"",
        "while read -r l; do IFS=: read -r a b <<< \"$l\"; done",
    ] {
        assert!(!runs_unseen(line), "{line}");
    }
    for (shell, line) in [
        ("bash", "v=$'a[\\x24\\x28cmd\\x290]'; echo $((v))"),
        ("zsh", "v=$'a[\\x24\\x28cmd\\x290]'; echo $((v))"),
    ] {
        assert!(runs_unseen_in(shell, line), "{shell}: {line}");
    }
}

/// Review 3: joins across quotes, `printf -v`, IFS joins, function
/// substitution, namerefs and zsh `$mapfile`.
#[test]
fn review_3_allow_list_and_new_sources() {
    for (shell, line) in [
        ("bash", "var=$(cat notes); n=va; m=\"$n\"r; echo $((m))"),
        ("bash", "var=$(cat notes); n=va; m=$n\"r\"; echo $((m))"),
        ("bash", "var=$(cat notes); n=va; m=$n\\r; echo $((m))"),
        ("bash", "var=$(cat notes); n=va; echo $(( $n\"r\" ))"),
        (
            "bash",
            "var=$(cat notes); printf -v n %s%s va r; echo $((n))",
        ),
        (
            "bash",
            "var=$(cat notes); a=(va r); IFS=; m=\"${a[*]}\"; echo $((m))",
        ),
        (
            "bash",
            "var=$(cat notes); set -- va r; IFS=; m=\"$*\"; echo $((m))",
        ),
        ("ksh", "v=${ cat notes; }; echo $((v))"),
        ("bash", "v=${| cat notes; }; echo $((v))"),
        ("bash", "v=$(cat notes); declare -n r=$v; echo $r"),
        (
            "zsh",
            "zmodload zsh/mapfile; v=$mapfile[notes]; echo $((v))",
        ),
    ] {
        assert!(runs_unseen_in(shell, line), "{shell}: {line}");
    }
    for line in [
        "v=$(cat f); n=3; m=$n; k=$((m+1)); rc=$?; echo $(( n + m + k + rc ))",
        "v=$(cat f); i=0; while (( i < 3 )); do i=$((i+1)); done",
        "v=$(cat f); w=${#v}; ((w > 2)) && echo long",
        "n=va; m=\"$n\"r; echo $((m))",
    ] {
        assert!(!runs_unseen(line), "{line}");
    }
}

/// Review 4: `$_`, zsh modifiers and subscripts, builtins that write named
/// variables, positional aliases, case conversion and late namerefs.
#[test]
fn review_4_routes() {
    for (shell, line) in [
        ("bash", ": \"$(cat notes)\"; echo $((_))"),
        ("bash", "v=$(cat notes); : \"$v\"; echo $((_))"),
        ("zsh", "var=$(cat notes); x=VAR; m=$x:l; echo $((m))"),
        ("zsh", "var=$(cat notes); x=var.c; m=$x:r; echo $((m))"),
        ("zsh", "var=$(cat notes); a=(va); m=$a[1]r; echo $((m))"),
        ("zsh", "var=$(cat notes); x=vaxx; m=$x[1,2]r; echo $((m))"),
        ("zsh", "var=$(cat notes); x=/v/var; echo $(( $x:t ))"),
        ("zsh", "var=$(cat notes); x=VAR; [[ $x:l -eq 1 ]]"),
        (
            "zsh",
            "v=$(cat notes); zstyle :x y \"$v\"; zstyle -s :x y m; echo $((m))",
        ),
        (
            "zsh",
            "v=$(cat notes); zformat -f m \"%a\" \"a:$v\"; echo $((m))",
        ),
        (
            "zsh",
            "v=$(cat notes); print -z -- \"$v\"; getln m; echo $((m))",
        ),
        (
            "zsh",
            "var=$(cat notes); typeset -T M m a; m=(v r); echo $((M))",
        ),
        ("zsh", "set -- \"$(cat notes)\"; echo $((argv[1]))"),
        ("zsh", "f(){ echo $((argv[1])); }; f \"$(cat notes)\""),
        (
            "bash",
            "shopt -s extdebug; f(){ echo $((BASH_ARGV[0])); }; f \"$(cat notes)\"",
        ),
        ("ksh", "var=$(cat notes); typeset -l m=VAR; echo $((m))"),
        ("bash", "var=$(cat notes); declare -u m; m=var; echo $((m))"),
        ("bash", "v=$(cat notes); declare -n r; r=$v; echo $r"),
    ] {
        assert!(runs_unseen_in(shell, line), "{shell}: {line}");
    }
    for line in [
        "out=$(cmd); arr=(1 2); n=${#arr[@]}; echo $((n-1))",
        "out=$(cmd); n=1; m=2; (( n + m )); (( n - m )); echo $(( n * m ))",
        "out=$(cmd); git checkout -; cat - notes; i=0; ((i++))",
        ": \"$(date)\"; i=0; ((i++))",
    ] {
        assert!(!runs_unseen(line), "{line}");
    }
}

/// Review 5: escaped or decoded `eval`/`alias` text, zsh integer
/// specials, attached option values, output only in arguments, anonymous
/// functions, and working-folder names.
#[test]
fn review_5_routes() {
    for (shell, line) in [
        ("bash", "v=$(cat notes); eval echo \\$\\(\\(v\\)\\)"),
        ("bash", "v=$(cat notes); eval \\(\\(v\\)\\)"),
        ("bash", "v=$(cat notes); eval $'echo $((\\x76))'"),
        ("bash", "v=$(cat notes); eval $'let \\x76'"),
        (
            "bash",
            "shopt -s expand_aliases; v=$(cat notes); alias L=$'let \\x76'\nL",
        ),
        ("zsh", "w=$(cat notes); UID=$w"),
        ("zsh", "w=$(cat notes); FUNCNEST=$w"),
        ("bash", "printf -vm %s \"$(cat notes)\"; echo $((m))"),
        ("bash", "read -raarr < notes; echo $((arr))"),
        ("zsh", "print -vm \"$(cat notes)\"; echo $((m))"),
        (
            "zsh",
            "zstyle :x y \"$(cat notes)\"; zstyle -s :x y m; echo $((m))",
        ),
        ("zsh", "print -z -- \"$(cat notes)\"; getln m; echo $((m))"),
        (
            "zsh",
            "setopt extendedglob; [[ \"$(cat notes)\" == (#b)(*) ]]; echo $((match[1]))",
        ),
        ("zsh", "zstat -A m +link lnk; echo $((m))"),
        ("zsh", "v=$(cat notes); () { echo $(( $2 )) } x \"$v\""),
        (
            "zsh",
            "v=$(cat notes); function { echo $(( $2 )) } x \"$v\"",
        ),
        (
            "zsh",
            "v=$(cat notes); functions[f]='echo $(( $1 ))'; f x \"$v\"",
        ),
        ("bash", "p=$(pwd); b=${p##*/}; echo $((b))"),
        ("bash", "p=$(git rev-parse --show-prefix); echo $((p))"),
    ] {
        assert!(runs_unseen_in(shell, line), "{shell}: {line}");
    }
    // Since the round-6 fallback, `eval` on a line holding command output is
    // unseen here too (end to end it already was: `eval` of command output).
    assert!(runs_unseen("eval \"$(ssh-agent -s)\" && i=0; ((i++))"));
    for line in [
        "alias ll='ls -l'; i=0; ((i++))",
        "f() { echo $(( $1 + 1 )); }; f 3",
        "echo $'a\\tb'; i=0; ((i++))",
    ] {
        assert!(!runs_unseen(line), "{line}");
    }
}

/// Review round 6 and the fallback it triggered: once the line holds
/// command output, arithmetic reads only variables whose every value is a
/// number or an arithmetic result, and `eval`/alias text is unseen.
#[test]
fn review_6_fallback_routes() {
    for (shell, line) in [
        // R6-1: `eval` text split by quotes or built in a variable.
        ("bash", "v=$(cat notes); eval \"(\"\"(v))\""),
        ("bash", "v=$(cat notes); eval 'a''[v]=1'"),
        ("bash", "v=$(cat notes); eval 'echo $''[v]'"),
        ("bash", "v=$(cat notes); c=\"(\"\"(v))\"; eval \"$c\""),
        ("ksh", "v=$(cat notes); c=\\(\\(v\\)\\); eval \"$c\""),
        // R6-2: an alias with escapes.
        (
            "bash",
            "shopt -s expand_aliases; v=$(cat notes); alias L=\\(\\(v\\)\\)\nL",
        ),
        // R6-3: `-v` inside an option cluster, also after a numeric value.
        ("zsh", "print -rv m \"$(cat notes)\"; echo $((m))"),
        ("zsh", "print -rvm \"$(cat notes)\"; echo $((m))"),
        ("zsh", "m=0; print -rvm \"$(cat notes)\"; echo $((m))"),
        ("zsh", "print -f '%s' -v m \"$(cat notes)\"; echo $((m))"),
        // R6-4: anonymous-function forms set the positional parameters.
        (
            "zsh",
            "v=$(cat notes); function () { echo $(( $2 )) } x \"$v\"",
        ),
        ("zsh", "v=$(cat notes); () (echo $(( $2 ))) x \"$v\""),
        (
            "zsh",
            "v=$(cat notes); function () (echo $(( $2 ))) x \"$v\"",
        ),
        // The fallback: variables the line does not show to be numbers.
        ("bash", "out=$(cat notes); echo $((m))"),
        ("bash", "out=$(cat notes); x=$1; echo $((x))"),
        ("bash", "out=$(cat notes); echo ${a[m]}"),
        ("zsh", "out=$(cat notes); echo $a[m]"),
        ("bash", "out=$(cat notes); a[m]=1"),
        ("bash", "out=$(cat notes); [[ m -eq 1 ]]"),
        ("bash", "out=$(cat notes); [[ -v a[m] ]]"),
        ("bash", "out=$(cat notes); test -v 'a[m]'"),
        ("zsh", "out=$(cat notes); printf '%d' m"),
        ("zsh", "out=$(cat notes); printf '%s %d' x m"),
        ("zsh", "out=$(cat notes); printf '%d' 'm%n'"),
        ("zsh", "m=1; out=$(cat notes); printf '%d' 'm%n'"),
        ("zsh", "fmt=%d; v=$(cat notes); printf \"$fmt\" \"$v\""),
        ("zsh", "out=$(cat notes); echo ${(l:m:)x}"),
        ("zsh", "out=$(cat notes); echo ${(P)m}"),
        ("zsh", "out=$(cat notes); echo ${(e)m}"),
        ("bash", "out=$(cat notes); echo ${!m}"),
        ("bash", "out=$(cat notes); let m"),
        ("bash", "out=$(cat notes); exit m"),
        ("bash", "out=$(cat notes); typeset -i n; n=m"),
        ("bash", "out=$(cat notes); x=hello; echo $((x))"),
        (
            "bash",
            "out=$(cat notes); arr=(a b); for i in \"${!arr[@]}\"; do echo $((i)); done",
        ),
        ("bash", "out=$(cat notes); bash -c '(''(m))'"),
        // Command output only as an argument now counts.
        ("zsh", "echo \"$(cat notes)\"; echo $((m))"),
    ] {
        assert!(runs_unseen_in(shell, line), "{shell}: {line}");
    }
    for line in [
        "out=$(cmd); for ((i=0; i<3; i++)); do echo $i; done",
        "out=$(cmd); n=$(nproc); make -j$((n+1))",
        "out=$(cmd); start=$(date +%s); echo $(( $(date +%s) - start ))",
        "out=$(cmd); grep -E 'foo[0-9]+' f; ls file[ab].txt",
        "out=$(cmd); [[ -f foo.txt && -n $out ]]",
        "out=$(cmd); printf '%5.1f%%\\n' 3",
        "out=$(cmd); printf 'Found %d\\n' 3",
        "out=$(cmd); i=0; j=$i; ((j++)); echo $((i+j))",
        "out=$(cmd); for i in 1 2 3; do echo $((i*2)); done",
        "out=$(cmd); echo ${out:0:80} ${#out}",
        "out=$(cmd); ulimit -n 1024; exit $?",
        "out=$(cmd); rc=$?; exit $rc",
        "out=$(cmd); arr=(1 2); for i in 0 1; do echo \"${arr[$i]}\"; done",
        "out=$(cmd); echo $((RANDOM % 10 + SECONDS + $# + ${#out}))",
        "out=$(cmd); n=0; while [[ $n -lt 3 ]]; do n=$((n+1)); done",
        "out=$(cmd); x=$((16#ff + 0x1f + 2)); echo $((x))",
        "i=0; ((i++)); echo $((m))",
    ] {
        assert!(!runs_unseen(line), "{line}");
    }
}

/// Review of the fallback (round 7): arithmetic that reads a name before
/// assigning it, `let -x`, zsh glob qualifiers, and facts printing their
/// arguments.
#[test]
fn review_7_fallback_routes() {
    for (shell, line) in [
        ("zsh", "set -sA i \"$(cat notes)\"; (( i++ ))"),
        ("ksh", "set -sA i \"$(cat notes)\"; (( i++ ))"),
        ("zsh", "set -sA i \"$(cat notes)\"; echo $(( i = i + 1 ))"),
        ("zsh", "v=$(cat notes); f g () { (( argv++ )); }; f \"$v\""),
        ("bash", "out=$(cat notes); echo $((i)); ((i=0))"),
        ("bash", "out=$(cat notes); echo $((i)); i=0"),
        ("bash", "out=$(cat notes); i=$(( i + 1 )); echo $((i))"),
        ("bash", "out=$(cat notes); echo \"$n\"; n=0; echo $((n))"),
        ("bash", "out=$(cat notes); (( i += 1 ))"),
        ("zsh", "set -sA foo \"$(cat notes)\"; let -foo"),
        ("zsh", "v=$(cat notes); f g () { let -argv; }; f \"$v\""),
        ("zsh", "w=$(cat notes); print -l /*([w])"),
        ("zsh", "w=$(cat notes); print -l /*([1,w])"),
        ("bash", "v=$(cat notes); n=$(date +\"$v\"); echo $((n))"),
        (
            "bash",
            "v=$(cat notes); n=$(git rev-parse --no-revs \"$v\"); echo $((n))",
        ),
        ("bash", "n=$(date +\"$(cat notes)\"); echo $((n))"),
    ] {
        assert!(runs_unseen_in(shell, line), "{shell}: {line}");
    }
    for line in [
        "out=$(cmd); for ((i=0; i<3; i++)); do echo $i; done",
        "out=$(cmd); for ((i=0, j=1; i<3; i++)); do echo $((i+j)); done",
        "out=$(cmd); ((total = 0)); ((total += 2)); echo $total",
        "out=$(cmd); ulimit -n 1024",
        "out=$(cmd); n=$(date +%s); echo $((n + 1))",
        "out=$(cmd); i=0; i=$(( i + 1 )); echo $((i))",
        "out=$(cmd); echo count; count=0; count=$((count+1))",
    ] {
        assert!(!runs_unseen(line), "{line}");
    }
}

/// The fallback stays linear on long chains of copies and long regions.
#[test]
fn review_6_fallback_stays_linear() {
    let mut line = String::from("out=$(cmd); v0=1;");
    for at in 1..20_000 {
        line.push_str(&format!(" v{at}=$v{};", at - 1));
    }
    line.push_str(" echo $((v19999))");
    let start = std::time::Instant::now();
    assert!(!runs_unseen(&line));
    // The helpers on long attacker text (the lexer's own cost on deep
    // nesting is main's, tracked separately).
    let deep = "$(".repeat(200_000);
    assert_eq!(without_substitutions(&deep).trim(), "");
    let ticks = "`".repeat(200_001);
    let _ = without_substitutions(&ticks);
    assert_eq!(
        arithmetic_assignments(&"i=1, ".repeat(100_000)).count(),
        100_000
    );
    // Without separators each value runs on: only the last one is read.
    assert_eq!(arithmetic_assignments(&"i=1 ".repeat(100_000)).count(), 1);
    assert!(numeric_references(&"$x + ".repeat(100_000)).is_some());
    let condition = format!("[[ {} ]]", "m -eq 1 && ".repeat(50_000));
    assert!(condition_unsafe(&condition, &|text: &str| names(text)
        .any(|name| name == "m")));
    let _ = format_text(&"%5d\\n".repeat(100_000));
    let flags = format!("${{({})x}}", "l:1:".repeat(50_000));
    let _ = flags_unsafe(&flags, &|_: &str| false);
    assert!(start.elapsed() < std::time::Duration::from_secs(20));
}
