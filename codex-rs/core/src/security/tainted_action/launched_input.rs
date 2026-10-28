//! #1276 items 1 and 5: input a launcher puts into the line it runs may be
//! the program itself (`printf git | xargs -I % % config core.pager sh`,
//! `find . -exec {} \;`, `xargs env < cmds`), code an interpreter or shell
//! runs (`xargs -I% python3 -c 'os.system("git config %")'`, `awk
//! 'BEGIN{system("%")}'`, `xargs sh -c`, `parallel 'eval {}'`), or, with no
//! command given, whole command lines (`parallel < cmds`, `xjobs < cmds`).
//! The classifier sees none of that input, so each asks as code it cannot
//! see, whatever feeds the launcher.
//!
//! Read loosely, so as to ask: an option a launcher's reading does not know
//! may take the next word as its value, so the command may start at that
//! word or at the one after it; past `MAX_LAUNCHED` launchers or launched
//! commands, or `MAX_DEPTH` shell scripts deep, any line asks. Input that
//! is only added after the command's words, and whole lines from input,
//! count only for a launcher at a program position (not `rg -n parallel`).

use super::indirect;
use super::invocation;
use super::invocation::Code;
use super::launched_git::FEEDERS;
use super::launched_git::FINDERS;
use super::launched_git::PARALLEL_FLAGS;
use super::launched_git::PARALLEL_VALUED;
use super::quoted;
use super::sed;
use super::shell::SHELLS;
use super::shell::basename;
use super::work;

/// Launcher words read, and launched commands judged, in one simple
/// command; more ask.
const MAX_LAUNCHED: usize = 16;
/// Shell scripts followed into (`xargs -I% sh -c 'sh -c "% x"'`); deeper
/// asks.
const MAX_DEPTH: usize = 4;
/// Replacement strings remembered for one launcher; past this many, any
/// word may hold one.
const MAX_REPLACEMENTS: usize = 8;

/// The word that stands for input a launcher adds after the command's own
/// words (`xargs env`, `fd -x sh -c`). No command line holds it.
const ADDED: &str = "\u{1}added-input\u{1}";

/// awk and its variants: the program is code (`awk 'BEGIN{system("x")}'`).
const AWKS: &[&str] = &["awk", "gawk", "mawk", "nawk", "busybox-awk"];

/// Commands that run their arguments as code in the shell they are in.
const EVALUATORS: &[&str] = &["eval", "source", ".", "trap"];

/// Programs that run the program named in their words but that the
/// command-word reading does not skip (`xargs -I% strace %`, `sudo %`).
const RUNNERS: &[&str] = &["sudo", "strace", "ltrace", "chroot", "doas"];

/// How a shell's script refers to its positional parameters, which a
/// launcher's input fills when it is in the shell's later arguments
/// (`find . -exec sh -c '"$1" x' _ {} \;`).
const POSITIONAL: &[&str] = &[
    "$@", "$*", "$0", "$1", "$2", "$3", "$4", "$5", "$6", "$7", "$8", "$9", "${", "$argv",
];

/// Where a launcher's input may go in the line it runs.
#[derive(Clone, Default)]
struct Fills {
    replacements: Vec<String>,
    /// More replacement strings than remembered.
    any: bool,
    /// The placeholders a finder or `parallel` replaces.
    placeholder: Placeholder,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Placeholder {
    #[default]
    None,
    /// `find`, `bfs`: `{}`.
    Find,
    /// `fd`, `parallel`, `rush`: `{}`, `{.}`, `{/}`, `{//}`, `{/.}`, `{#}`,
    /// `{%}`, `{1}`, `{=perl=}`, `{+.}`, rush's `{:}`, `{^x}`, `{@re}`,
    /// `{?}`; not awk's `{print}` or `{++n}`.
    Braced,
}

impl Placeholder {
    fn in_text(self, text: &str) -> bool {
        match self {
            Self::None => false,
            Self::Find => text.contains("{}"),
            Self::Braced => text.match_indices('{').any(|(at, _)| {
                work::step();
                let after = &text[at + 1..];
                after.starts_with(|ch: char| ch.is_ascii_digit() || "}./#%=:^@?".contains(ch))
                    || after.starts_with("+.")
                    || after.starts_with("+/")
            }),
        }
    }
}

impl Fills {
    fn new(placeholder: Placeholder) -> Self {
        Self {
            placeholder,
            ..Self::default()
        }
    }

    fn positional() -> Self {
        Self {
            replacements: POSITIONAL.iter().map(|name| (*name).to_string()).collect(),
            ..Self::default()
        }
    }

    fn push(&mut self, replacement: &str) {
        if replacement.is_empty() {
            return;
        }
        if self.replacements.len() == MAX_REPLACEMENTS {
            self.any = true;
        } else {
            self.replacements.push(replacement.to_string());
        }
    }

    /// Whether input may be put in `text`, added after the words too.
    fn fill(&self, text: &str) -> bool {
        text.contains(ADDED) || self.placed(text)
    }

    /// Whether input may be put in `text` where it is.
    fn placed(&self, text: &str) -> bool {
        self.any
            || self.placeholder.in_text(text)
            || self
                .replacements
                .iter()
                .any(|replacement| text.contains(replacement.as_str()))
    }
}

/// One command a launcher runs.
enum Launched {
    /// The launcher runs its input as whole command lines.
    Lines,
    /// `parallel`/`rush`: the words are joined into one shell line.
    /// `piped`: the input goes to its standard input (`--pipe`).
    Line {
        words: Vec<String>,
        fills: Fills,
        piped: bool,
    },
    Words(Vec<String>, Fills),
}

/// Whether input a launcher in `texts` (one simple command's words, as the
/// quote-aware reading gives them) may run as a program or as code.
pub(super) fn input_runs(texts: &[String]) -> bool {
    let lower: Vec<String> = texts.iter().map(|text| text.to_lowercase()).collect();
    let Some(programs) = program_positions(texts) else {
        return true;
    };
    let mut launched = Vec::new();
    let mut launchers = 0;
    for (at, word) in lower.iter().enumerate() {
        work::step();
        let name = basename(word);
        let program = programs[at];
        let args = &texts[at + 1..];
        match name {
            _ if FEEDERS.contains(&name) => xargs(args, name == "xjobs", program, &mut launched),
            "parallel" | "env_parallel" => parallel(args, program, &mut launched),
            "rush" => rush(args, &mut launched),
            _ if FINDERS.contains(&name) => {
                finder(
                    args,
                    matches!(name, "fd" | "fdfind"),
                    program,
                    &mut launched,
                );
            }
            _ => continue,
        }
        launchers += 1;
        if launchers > MAX_LAUNCHED || launched.len() > MAX_LAUNCHED {
            return true;
        }
    }
    launched.iter().any(|launched| {
        work::step();
        match launched {
            Launched::Lines => true,
            Launched::Line {
                words,
                fills,
                piped,
            } => {
                let line = words.join(" ");
                (*piped && stdin_code(&line)) || line_runs(&line, fills, /*depth*/ 0)
            }
            Launched::Words(words, fills) => words_run(words, fills, /*depth*/ 0),
        }
    })
}

/// For each word, whether it is where a program may start: a command word,
/// a wrapper's, or one an exec wrapper or `-exec` runs. `None` past the
/// places read.
fn program_positions(texts: &[String]) -> Option<Vec<bool>> {
    let (starts, more) = sed::command_starts(texts);
    if more {
        return None;
    }
    let mut programs = vec![false; texts.len()];
    for start in starts {
        work::step();
        for at in indirect::programs(&texts[start..]) {
            work::step();
            programs[start + at] = true;
        }
    }
    Some(programs)
}

/// The words of a command a launcher runs, from `at`, with the word for
/// input it adds after them (`added`).
fn command_words(words: &[String], added: bool) -> Vec<String> {
    let mut command = words.to_vec();
    if added {
        command.push(ADDED.to_string());
    }
    command
}

/// `xargs`/`xjobs` and its options: the command starts at the first word
/// that is not an option or an option's value (or at the word after an
/// option it does not know). Without a replacement string, input is added
/// after the command's words. xjobs without a command, or with `-s
/// script`, runs its input lines as commands. `program`: the launcher is
/// at a program position.
fn xargs(words: &[String], xjobs: bool, program: bool, launched: &mut Vec<Launched>) {
    const XARGS_VALUED: &[&str] = &[
        "arg-file",
        "delimiter",
        "max-args",
        "max-procs",
        "max-chars",
        "process-slot-var",
    ];
    const XARGS_ATTACHED: &[&str] = &["eof", "replace", "max-lines"];
    const XARGS_FLAGS: &[&str] = &[
        "null",
        "interactive",
        "no-run-if-empty",
        "verbose",
        "exit",
        "open-tty",
        "show-limits",
        "help",
        "version",
    ];
    let mut fills = Fills::default();
    // The word before the command is an option this reading does not know.
    let mut unsure = false;
    let mut at = 0;
    while let Some(word) = words.get(at) {
        work::step();
        if word == "--" {
            at += 1;
            unsure = false;
            break;
        }
        if !word.starts_with('-') || word == "-" {
            break;
        }
        unsure = false;
        if let Some(long) = word.strip_prefix("--").filter(|_| !xjobs) {
            let (option, value) = long
                .split_once('=')
                .map_or((long, None), |(option, value)| (option, Some(value)));
            let pick = |names: &[&'static str]| names.iter().any(|name| name.starts_with(option));
            if option.is_empty() {
                unsure = value.is_none();
            } else if "replace".starts_with(option) && option.starts_with('r') {
                fills.push(value.unwrap_or("{}"));
            } else if pick(XARGS_VALUED) {
                at += usize::from(value.is_none());
            } else if !(pick(XARGS_ATTACHED) || pick(XARGS_FLAGS)) {
                unsure = value.is_none();
            }
            at += 1;
            continue;
        }
        let Some(group) = word.strip_prefix('-').filter(|group| !group.is_empty()) else {
            break;
        };
        at += 1;
        for (index, letter) in group.char_indices() {
            work::step();
            let rest = &group[index + letter.len_utf8()..];
            let valued = if xjobs { "jlqv" } else { "adEIJLnPRsS" };
            if xjobs && letter == 's' {
                if program {
                    launched.push(Launched::Lines);
                }
                return;
            }
            if valued.contains(letter) {
                let value = if rest.is_empty() {
                    at += 1;
                    words.get(at - 1).map_or("", String::as_str)
                } else {
                    rest
                };
                if !xjobs && matches!(letter, 'I' | 'J') {
                    fills.push(value);
                }
                break;
            }
            if !xjobs && matches!(letter, 'e' | 'i' | 'l') {
                if letter == 'i' {
                    fills.push(if rest.is_empty() { "{}" } else { rest });
                }
                break;
            }
            let flags = if xjobs { "01dnptV" } else { "0oprtx" };
            if !flags.contains(letter) {
                unsure = rest.is_empty();
                break;
            }
        }
    }
    if at >= words.len() {
        if xjobs && program {
            launched.push(Launched::Lines);
        }
        return;
    }
    // Without a replacement string, input words go after the command's.
    let added = program && fills.replacements.is_empty();
    if unsure && at + 1 < words.len() {
        launched.push(Launched::Words(
            command_words(&words[at + 1..], added),
            fills.clone(),
        ));
    }
    launched.push(Launched::Words(command_words(&words[at..], added), fills));
}

/// The replacement string a GNU `parallel` option sets, by its name
/// (`-I`, `--extensionreplace`/`--er`, `--bnr`, `--dnr`, `--bner`,
/// `--rpl 'R perl'`, ...).
fn sets_replacement(option: &str) -> bool {
    matches!(
        option,
        "-I" | "--er" | "--bnr" | "--dnr" | "--bner" | "--rpl" | "--parens"
    ) || (option.contains("replace") && option != "--max-replace-args")
}

/// GNU `parallel` (`env_parallel`): the command words before the first
/// `:::`-style argument are joined and run as a shell line, with input put
/// where `{...}` placeholders and replacement strings are, or added at the
/// end when the line holds none. Without a command it runs its input lines
/// as commands, unless they are only its own literal `:::` arguments (judged
/// as lines elsewhere).
fn parallel(words: &[String], program: bool, launched: &mut Vec<Launched>) {
    let mut fills = Fills::new(Placeholder::Braced);
    let mut unsure = false;
    let mut file_input = false;
    let mut piped = false;
    let mut at = 0;
    while let Some(word) = words.get(at) {
        work::step();
        if word.starts_with(":::") || !word.starts_with('-') || word == "-" {
            break;
        }
        at += 1;
        unsure = false;
        if word == "--" {
            break;
        }
        let (option, value) = word
            .split_once('=')
            .map_or((word.as_str(), None), |(option, value)| {
                (option, Some(value))
            });
        file_input |= option.starts_with("-a") || option.starts_with("--a");
        piped |= matches!(option, "--pipe" | "--pipepart");
        if option == "--parens" {
            // Other placeholder brackets.
            fills.any = true;
        }
        // `-I%` attached too.
        let attached_replacement = option.strip_prefix("-I").filter(|rest| !rest.is_empty());
        if attached_replacement.is_some() || sets_replacement(option) {
            let value = match (attached_replacement, value) {
                (Some(replacement), _) => replacement,
                (None, Some(value)) => value,
                (None, None) => {
                    at += 1;
                    words.get(at - 1).map_or("", String::as_str)
                }
            };
            fills.push(value.split_whitespace().next().unwrap_or(""));
            continue;
        }
        let counted = ["-j", "-P", "-N", "-n"].iter().any(|short| {
            option.strip_prefix(short).is_some_and(|count| {
                !count.is_empty()
                    && count
                        .chars()
                        .all(|ch| ch.is_ascii_digit() || "+-%".contains(ch))
            })
        });
        if PARALLEL_VALUED.contains(&option) {
            at += usize::from(value.is_none());
        } else if !(counted || PARALLEL_FLAGS.contains(&option)) {
            // An option this reading does not know: if it takes a value,
            // that is the next word, which may set a replacement string.
            unsure = value.is_none();
        }
    }
    let end = words[at..]
        .iter()
        .position(|word| word.starts_with(":::"))
        .map_or(words.len(), |offset| at + offset);
    if at == end {
        let literal_only = end < words.len()
            && !file_input
            && words[end..].iter().all(|word| {
                work::step();
                !word.starts_with("::::")
                    && !word.contains(['$', '`'])
                    && !word.contains(super::shell::UNSEEN_OUTPUT)
            });
        if program && !literal_only {
            launched.push(Launched::Lines);
        }
        return;
    }
    if unsure && at + 1 < end {
        let mut with_value = fills.clone();
        with_value.push(words[at].split_whitespace().next().unwrap_or(""));
        launched.push(parallel_line(&words[at + 1..end], with_value, piped));
    }
    launched.push(parallel_line(&words[at..end], fills, piped));
}

/// A `parallel` line: input goes where its placeholders are, else (unless
/// piped) after its words.
fn parallel_line(words: &[String], fills: Fills, piped: bool) -> Launched {
    let placed = words.iter().any(|word| {
        work::step();
        fills.placed(word)
    });
    let mut words = words.to_vec();
    if !placed && !piped {
        words.push(ADDED.to_string());
    }
    Launched::Line {
        words,
        fills,
        piped,
    }
}

/// `rush [flags] command`: its flags may come anywhere; the other words
/// are the command line it runs with a shell, with input where its
/// placeholders are (and `-v name=value` names more: `{name}`). Without a
/// command it prints its input.
fn rush(words: &[String], launched: &mut Vec<Launched>) {
    /// rush's long flags that take a value.
    const VALUED: &[&str] = &[
        "jobs",
        "retries",
        "retry-interval",
        "timeout",
        "succeed-cmd-file",
        "record-delimiter",
        "nrecords",
        "field-delimiter",
        "records-join-sep",
        "infile",
        "out-file",
        "assign",
        "trim",
        "delay",
        "load",
        "memfree",
        "cleanup-time",
        "no-stop-exes",
        "no-kill-exes",
    ];
    let mut fills = Fills::new(Placeholder::Braced);
    let mut piped = false;
    let mut line = Vec::new();
    let mut at = 0;
    while let Some(word) = words.get(at) {
        work::step();
        at += 1;
        if word == "--" {
            line.extend(words[at..].iter().cloned());
            break;
        }
        let assigned = |fills: &mut Fills, value: &str| {
            if let Some((name, _)) = value.split_once('=') {
                fills.push(&format!("{{{name}}}"));
            }
        };
        if let Some(long) = word.strip_prefix("--") {
            let (option, value) = long
                .split_once('=')
                .map_or((long, None), |(option, value)| (option, Some(value)));
            piped |= option == "pipe";
            if VALUED.contains(&option) {
                let value = value.or_else(|| {
                    at += 1;
                    words.get(at - 1).map(String::as_str)
                });
                if option == "assign" {
                    assigned(&mut fills, value.unwrap_or(""));
                }
            }
            continue;
        }
        let Some(group) = word.strip_prefix('-').filter(|group| !group.is_empty()) else {
            line.push(word.clone());
            continue;
        };
        for (index, letter) in group.char_indices() {
            work::step();
            if !"jrtCDndJiovT".contains(letter) {
                continue;
            }
            let rest = &group[index + letter.len_utf8()..];
            let value = if rest.is_empty() {
                at += 1;
                words.get(at - 1).map_or("", String::as_str)
            } else {
                rest
            };
            if letter == 'v' {
                assigned(&mut fills, value);
            }
            break;
        }
    }
    if !line.is_empty() {
        launched.push(Launched::Line {
            words: line,
            fills,
            piped,
        });
    }
}

/// `find`/`bfs` `-exec`/`-execdir`/`-ok`/`-okdir`, and `fd` `-x`/`-X`/
/// `--exec`/`--exec-batch` (in a group too: `-HX`; or attached: `--exec=cmd`,
/// `-xcmd`): the command each runs, up to its `;` or `+`. fd adds the path
/// after a command that holds no placeholder.
fn finder(words: &[String], fd: bool, program: bool, launched: &mut Vec<Launched>) {
    let placeholder = if fd {
        Placeholder::Braced
    } else {
        Placeholder::Find
    };
    let mut at = 0;
    while let Some(word) = words.get(at) {
        work::step();
        at += 1;
        let (starts, attached) = if fd {
            fd_exec(word)
        } else {
            (
                matches!(word.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir"),
                None,
            )
        };
        if !starts {
            continue;
        }
        let end = words[at..]
            .iter()
            .position(|word| matches!(word.as_str(), ";" | "+"))
            .map_or(words.len(), |offset| at + offset);
        let mut command: Vec<String> = attached.into_iter().map(str::to_string).collect();
        command.extend(words[at..end].iter().cloned());
        let fills = Fills::new(placeholder);
        let placed = command.iter().any(|word| {
            work::step();
            fills.placed(word)
        });
        if fd && program && !placed {
            command.push(ADDED.to_string());
        }
        launched.push(Launched::Words(command, fills));
        if launched.len() > MAX_LAUNCHED {
            return;
        }
        at = end;
    }
}

/// Whether an `fd` word starts an exec option, and the command it holds
/// attached. In a group of short options, the letters after one that takes
/// a value are that value (`-tx` is the type `x`).
fn fd_exec(word: &str) -> (bool, Option<&str>) {
    if let Some(long) = word.strip_prefix("--") {
        if let Some(command) = ["exec=", "exec-batch="]
            .iter()
            .find_map(|option| long.strip_prefix(option))
        {
            return (true, Some(command).filter(|command| !command.is_empty()));
        }
        return (matches!(long, "exec" | "exec-batch"), None);
    }
    let Some(group) = word.strip_prefix('-') else {
        return (false, None);
    };
    for (at, letter) in group.char_indices() {
        work::step();
        if matches!(letter, 'x' | 'X') {
            let command = &group[at + 1..];
            let command = command.strip_prefix('=').unwrap_or(command);
            return (true, Some(command).filter(|command| !command.is_empty()));
        }
        // fd's short options that take a value.
        if "tedEjScoD".contains(letter) {
            break;
        }
    }
    (false, None)
}

/// Whether input reaches the program `words` run (or a wrapper's, or one
/// an exec wrapper or `-exec` runs: `xargs -I{} env {} config`), code an
/// evaluator, interpreter or awk runs, or a shell's script, or the
/// positional parameters a shell's script runs.
fn words_run(words: &[String], fills: &Fills, depth: usize) -> bool {
    let Some(programs) = program_positions(words) else {
        return true;
    };
    if words
        .iter()
        .zip(&programs)
        .any(|(word, program)| *program && fills.placed(word))
    {
        return true;
    }
    // `env -S'% x'`: env splits the string into the words it runs, before
    // the words after it (so the input added after them is not the
    // program).
    let command = indirect::command_word(words);
    // env's options end at its command (`env git log --stat` has none).
    let options = command.as_ref().map_or(words.len(), |(at, _)| at + 1);
    let mut env = false;
    let mut env_split = false;
    let mut split_next = false;
    for (at, word) in words[..options].iter().enumerate() {
        work::step();
        let value = if std::mem::take(&mut split_next) {
            Some(word.as_str())
        } else if let Some(value) = word.strip_prefix("-S") {
            Some(value)
        } else if word.starts_with("--s") {
            word.split_once('=').map(|(_, value)| value)
        } else {
            None
        };
        if env && let Some(value) = value {
            env_split = true;
            if fills.fill(value) {
                if depth > MAX_DEPTH {
                    return true;
                }
                let mut split: Vec<String> = value.split_whitespace().map(str::to_string).collect();
                split.extend(words[at + 1..].iter().cloned());
                if words_run(&split, fills, depth + 1) {
                    return true;
                }
            }
        }
        env |= basename(&word.to_lowercase()) == "env";
        split_next = env
            && (word == "-S" || (word.starts_with("--s") && !word.contains('=') && word.len() > 3));
    }
    // Input added after words that only wrap is the program (`xargs env`).
    if !env_split
        && command.as_ref().is_none_or(|(at, _)| words[*at] == ADDED)
        && words.iter().any(|word| word == ADDED)
    {
        return true;
    }
    let Some((index, name)) = command else {
        return false;
    };
    let args = &words[index + 1..];
    if EVALUATORS.contains(&name.as_str()) {
        return args.iter().any(|arg| fills.fill(arg));
    }
    if RUNNERS.contains(&name.as_str()) {
        // The program they run: their first word that is not an option
        // (after chroot's folder).
        return args
            .iter()
            .filter(|arg| !arg.starts_with('-'))
            .nth(usize::from(name == "chroot"))
            .is_some_and(|arg| fills.fill(arg));
    }
    if AWKS.contains(&name.as_str()) {
        let programs = awk_programs(args);
        return if programs.is_empty() {
            // No program yet: input added after the words is the program.
            args.last().is_some_and(|arg| arg == ADDED)
        } else {
            programs.iter().any(|program| fills.fill(program))
        };
    }
    let shell = SHELLS.contains(&name.as_str());
    match invocation::parse(&name, args).map(|invocation| invocation.code) {
        // `xargs sh -c`: the added input is the code.
        Some(Code::Inline(None)) => args.last().is_some_and(|arg| arg == ADDED),
        Some(Code::Inline(Some(code))) => {
            if fills.fill(code) {
                return !shell || line_runs(code, fills, depth + 1);
            }
            // `sh -c '"$1" x' _ {}`: input in the shell's arguments.
            let after = args
                .iter()
                .position(|arg| arg.ends_with(code))
                .map_or(args.len(), |at| at + 1);
            shell
                && args[after..].iter().any(|arg| fills.fill(arg))
                && line_runs(code, &Fills::positional(), depth + 1)
        }
        _ => false,
    }
}

/// Whether input reaches a program or code in the commands of a shell line
/// that holds it.
fn line_runs(line: &str, fills: &Fills, depth: usize) -> bool {
    if !fills.fill(line) {
        return false;
    }
    if depth > MAX_DEPTH {
        return true;
    }
    let reading = quoted::commands(line);
    if reading.too_deep || reading.unsure {
        return true;
    }
    reading.commands.iter().any(|command| {
        work::step();
        let texts: Vec<String> = command.words.iter().map(|word| word.text.clone()).collect();
        // A shell or interpreter reading code from its input (`echo {} |
        // sh`, `bash <<< {}`), or a command named by a variable (`x={};
        // $x`), in a line that holds the input.
        indirect::command_word(&texts).is_some_and(|(_, name)| name.contains(['$', '`']))
            || stdin_reader(&texts)
            || words_run(&texts, fills, depth)
    })
}

/// Whether a shell line runs a shell or interpreter that reads code from
/// its standard input (`parallel --pipe bash`).
fn stdin_code(line: &str) -> bool {
    let reading = quoted::commands(line);
    reading.too_deep
        || reading.unsure
        || reading.commands.iter().any(|command| {
            work::step();
            let texts: Vec<String> = command.words.iter().map(|word| word.text.clone()).collect();
            stdin_reader(&texts)
        })
}

fn stdin_reader(texts: &[String]) -> bool {
    indirect::invocation(texts).is_some_and(|invocation| matches!(invocation.code, Code::Stdin))
}

/// awk's program texts: `-e`/`--source` values, or else its first operand
/// unless a program file is given (`-f`, `--file`, gawk `-E`).
fn awk_programs(args: &[String]) -> Vec<&str> {
    /// gawk's long options that take a value.
    const VALUED: &[&str] = &[
        "source",
        "file",
        "exec",
        "include",
        "load",
        "assign",
        "field-separator",
    ];
    let mut programs = Vec::new();
    let mut file = false;
    let mut at = 0;
    while let Some(arg) = args.get(at) {
        work::step();
        at += 1;
        if arg == "--" {
            break;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (option, value) = long
                .split_once('=')
                .map_or((long, None), |(option, value)| (option, Some(value)));
            let Some(name) = VALUED
                .iter()
                .find(|name| option.len() > 1 && name.starts_with(option))
            else {
                continue;
            };
            let value = value.or_else(|| {
                at += 1;
                args.get(at - 1).map(String::as_str)
            });
            match *name {
                "source" => programs.extend(value),
                "file" | "exec" => file = true,
                _ => {}
            }
            continue;
        }
        let Some(group) = arg.strip_prefix('-').filter(|group| !group.is_empty()) else {
            at -= 1;
            break;
        };
        for (index, letter) in group.char_indices() {
            work::step();
            let rest = &group[index + letter.len_utf8()..];
            if !"FvfEeilW".contains(letter) {
                continue;
            }
            let value = if rest.is_empty() {
                at += 1;
                args.get(at - 1).map_or("", String::as_str)
            } else {
                rest
            };
            match letter {
                'e' => programs.push(value),
                'f' | 'E' => file = true,
                _ => {}
            }
            break;
        }
    }
    if programs.is_empty()
        && !file
        && let Some(program) = args.get(at).filter(|program| *program != ADDED)
    {
        programs.push(program);
    }
    programs
}
