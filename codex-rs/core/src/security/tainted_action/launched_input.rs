//! #1276 items 1 and 5: input a launcher puts into the line it runs may be
//! the program itself (`printf git | xargs -I % % config core.pager sh`,
//! `find . -exec {} \;`), code an interpreter runs (`xargs -I% python3 -c
//! 'os.system("git config %")'`, `awk 'BEGIN{system("%")}'`), or, with no
//! command given, whole command lines (`parallel < cmds`, `xjobs < cmds`).
//! The classifier sees none of that input, so each asks as code it cannot
//! see, whatever feeds the launcher.
//!
//! Read loosely, so as to ask: an option a launcher's reading does not know
//! may take the next word as its value, so that word and the one after it
//! may each be the command; past `MAX_LAUNCHED` launched commands, or
//! `MAX_DEPTH` shell scripts deep, any line asks.

use super::indirect;
use super::invocation;
use super::invocation::Code;
use super::launched_git::FEEDERS;
use super::launched_git::FINDERS;
use super::launched_git::PARALLEL_FLAGS;
use super::launched_git::PARALLEL_VALUED;
use super::launched_git::PARALLELS;
use super::quoted;
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

/// awk and its variants: the program is code (`awk 'BEGIN{system("x")}'`).
const AWKS: &[&str] = &["awk", "gawk", "mawk", "nawk", "busybox-awk"];

/// Where a launcher's input may go in the line it runs.
#[derive(Default)]
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
    /// `{%}`, `{1}`, `{=perl=}` and the like; not awk's `{print}`.
    Braced,
}

impl Placeholder {
    fn in_text(self, text: &str) -> bool {
        match self {
            Self::None => false,
            Self::Find => text.contains("{}"),
            Self::Braced => text.match_indices('{').any(|(at, _)| {
                work::step();
                text[at + 1..]
                    .starts_with(|ch: char| ch.is_ascii_digit() || "}./#%=+-".contains(ch))
            }),
        }
    }
}

impl Fills {
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

    /// Whether input may be put in `text`.
    fn fill(&self, text: &str) -> bool {
        self.any
            || self.placeholder.in_text(text)
            || self
                .replacements
                .iter()
                .any(|replacement| text.contains(replacement.as_str()))
    }
}

/// One command a launcher runs: its words (from where the command may
/// start), and where input goes in them.
enum Launched {
    /// The launcher runs its input as whole command lines.
    Lines,
    /// `parallel`/`rush`: the words are joined into one shell line.
    Line(Vec<String>, Fills),
    Words(Vec<String>, Fills),
}

/// Whether input a launcher in `texts` (one simple command's words, as the
/// quote-aware reading gives them) may run as a program or as code.
pub(super) fn input_runs(texts: &[String]) -> bool {
    let lower: Vec<String> = texts.iter().map(|text| text.to_lowercase()).collect();
    let mut launched = Vec::new();
    let mut launchers = 0;
    for (at, word) in lower.iter().enumerate() {
        work::step();
        let name = basename(word);
        if FEEDERS.contains(&name) {
            xargs(&texts[at + 1..], name == "xjobs", &mut launched);
        } else if PARALLELS.contains(&name) {
            parallel(&texts[at + 1..], name == "rush", &mut launched);
        } else if FINDERS.contains(&name) {
            finder(
                &texts[at + 1..],
                matches!(name, "fd" | "fdfind"),
                &mut launched,
            );
        } else {
            continue;
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
            Launched::Line(words, fills) => line_runs(&words.join(" "), fills, /*depth*/ 0),
            Launched::Words(words, fills) => words_run(words, fills, /*depth*/ 0),
        }
    })
}

/// `xargs`/`xjobs` and its options: the command starts at the first word
/// that is not an option or an option's value (or at either of two words
/// after an option it does not know). xjobs without a command, or with
/// `-s script`, runs its input lines as commands.
fn xargs(words: &[String], xjobs: bool, launched: &mut Vec<Launched>) {
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
    let mut unsure = false;
    let mut at = 0;
    while let Some(word) = words.get(at) {
        work::step();
        if word == "--" {
            at += 1;
            break;
        }
        if let Some(long) = word.strip_prefix("--").filter(|_| !xjobs) {
            let (option, value) = long
                .split_once('=')
                .map_or((long, None), |(o, v)| (o, Some(v)));
            let pick = |names: &[&'static str]| names.iter().any(|name| name.starts_with(option));
            if option.is_empty() {
                unsure = true;
            } else if "replace".starts_with(option) && option.starts_with('r') {
                fills.push(value.unwrap_or("{}"));
            } else if pick(XARGS_VALUED) {
                at += usize::from(value.is_none());
            } else if !(pick(XARGS_ATTACHED) || pick(XARGS_FLAGS)) {
                unsure = true;
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
                launched.push(Launched::Lines);
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
                unsure = true;
                break;
            }
        }
    }
    if at >= words.len() {
        if xjobs {
            launched.push(Launched::Lines);
        }
        return;
    }
    if unsure && at + 1 < words.len() {
        launched.push(Launched::Words(
            words[at + 1..].to_vec(),
            fills_copy(&fills),
        ));
    }
    launched.push(Launched::Words(words[at..].to_vec(), fills));
}

fn fills_copy(fills: &Fills) -> Fills {
    Fills {
        replacements: fills.replacements.clone(),
        any: fills.any,
        placeholder: fills.placeholder,
    }
}

/// GNU `parallel` (`env_parallel`) and `rush`: the command words before the
/// first `:::`-style argument are joined and run as a shell line, with
/// input put where `{...}` placeholders and replacement strings are.
/// `parallel` without a command runs its input lines as commands, unless
/// they are only its own literal `:::` arguments (which are judged as
/// lines elsewhere).
fn parallel(words: &[String], rush: bool, launched: &mut Vec<Launched>) {
    let mut fills = Fills {
        placeholder: Placeholder::Braced,
        ..Fills::default()
    };
    let mut unsure = false;
    let mut file_input = false;
    let mut at = 0;
    while let Some(word) = words.get(at) {
        work::step();
        if word.starts_with(":::") || !word.starts_with('-') || word == "-" {
            break;
        }
        at += 1;
        if word == "--" {
            break;
        }
        let option = word
            .split_once('=')
            .map_or(word.as_str(), |(option, _)| option);
        let attached = word.contains('=');
        file_input |= option == "-a" || option.starts_with("--a") || option.starts_with("-a");
        if option == "-I"
            || (option.contains("replace") && option != "--max-replace-args")
            || option.starts_with("--rpl")
        {
            // `-I R`, `--extensionreplace R`, `--rpl 'R perl'`.
            let value = if attached {
                word.split_once('=').map_or("", |(_, value)| value)
            } else {
                at += 1;
                words.get(at - 1).map_or("", String::as_str)
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
            at += usize::from(!attached);
        } else if !(counted || PARALLEL_FLAGS.contains(&option)) {
            // An option this reading does not know: its value, if it takes
            // one, may be a replacement string or the command.
            unsure = true;
            if let Some(next) = words.get(at).filter(|_| !attached) {
                fills.push(next.split_whitespace().next().unwrap_or(""));
            }
        }
    }
    let end = words[at..]
        .iter()
        .position(|word| word.starts_with(":::"))
        .map_or(words.len(), |offset| at + offset);
    if at == end {
        // No command: `rush` echoes its input; `parallel` runs it.
        let literal_only = end < words.len()
            && !file_input
            && words[end..].iter().all(|word| {
                work::step();
                !word.starts_with("::::")
                    && !word.contains(['$', '`'])
                    && !word.contains(super::shell::UNSEEN_OUTPUT)
            });
        if !rush && !literal_only {
            launched.push(Launched::Lines);
        }
        return;
    }
    if unsure && at + 1 < end {
        launched.push(Launched::Line(
            words[at + 1..end].to_vec(),
            fills_copy(&fills),
        ));
    }
    launched.push(Launched::Line(words[at..end].to_vec(), fills));
}

/// `find`/`bfs` `-exec`/`-execdir`/`-ok`/`-okdir`, and `fd` `-x`/`-X`/
/// `--exec`/`--exec-batch` (in a group too: `-HX`; or attached: `--exec=cmd`,
/// `-xcmd`): the command each runs, up to its `;` or `+`.
fn finder(words: &[String], fd: bool, launched: &mut Vec<Launched>) {
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
        launched.push(Launched::Words(
            command,
            Fills {
                placeholder,
                ..Fills::default()
            },
        ));
        if launched.len() > MAX_LAUNCHED {
            return;
        }
        at = end;
    }
}

/// Whether an `fd` word starts an exec option, and the command it holds
/// attached.
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
    match group.find(['x', 'X']) {
        Some(at) => {
            let command = &group[at + 1..];
            let command = command.strip_prefix('=').unwrap_or(command);
            (true, Some(command).filter(|command| !command.is_empty()))
        }
        None => (false, None),
    }
}

/// Whether input reaches the program `words` run (or a wrapper before it:
/// `xargs -I{} env {} config`), an interpreter's inline code or awk's
/// program, or, in a shell's script, any of these.
fn words_run(words: &[String], fills: &Fills, depth: usize) -> bool {
    if indirect::programs(words)
        .iter()
        .any(|&at| fills.fill(&words[at]))
    {
        return true;
    }
    let Some((index, name)) = indirect::command_word(words) else {
        return false;
    };
    let args = &words[index + 1..];
    if AWKS.contains(&name.as_str()) {
        return awk_programs(args).iter().any(|program| fills.fill(program));
    }
    match invocation::parse(&name, args).map(|invocation| invocation.code) {
        Some(Code::Inline(Some(code))) if fills.fill(code) => {
            !SHELLS.contains(&name.as_str()) || line_runs(code, fills, depth + 1)
        }
        _ => false,
    }
}

/// Whether input reaches a program or code in the commands of a shell line.
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
        words_run(&texts, fills, depth)
    })
}

/// awk's program texts: `-e`/`--source` values, or else its first operand
/// unless a program file is given (`-f`, `--file`, gawk `-E`).
fn awk_programs(args: &[String]) -> Vec<&str> {
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
            let (option, value) = long
                .split_once('=')
                .map_or((long, None), |(o, v)| (o, Some(v)));
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
            if !"FvfEeil".contains(letter) {
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
        && let Some(program) = args.get(at)
    {
        programs.push(program);
    }
    programs
}
