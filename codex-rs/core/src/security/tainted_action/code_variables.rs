//! #1142: settings that make git (or a pager or editor it starts) run a
//! command. git runs the value of `GIT_PAGER`, `PAGER`, `GIT_EDITOR`,
//! `GIT_SSH_COMMAND`, `GIT_EXTERNAL_DIFF`... through a shell, reads more such
//! settings from `GIT_CONFIG_*` and runs programs from `GIT_EXEC_PATH`; a
//! `-c key=value` or `--config-env` override sets any config key, many of
//! which name a command (`core.pager`, `alias.x=!cmd`, `diff.x.textconv`).
//! Setting one is code the classifier does not read.
//!
//! This over-approximates on purpose: any word that names such a variable
//! other than by reading it (`GIT_PAGER=x`, `export PAGER`, `printf -v
//! PAGER`, `read EDITOR`, `declare -n r=PAGER`, `${PAGER:=x}`) counts, in
//! any command, since a variable set in a persistent shell is used by a
//! later command; so does a builtin that names a variable the classifier
//! cannot see (`printf -v "$name"`).

use super::shell;
use super::shell::SimpleCommand;
use super::work;

/// Variables whose value git, a pager or an editor runs as a command or
/// that point git at programs or config to run.
const CODE_VARIABLES: &[&str] = &[
    "GIT_PAGER",
    "PAGER",
    "MANPAGER",
    "GIT_EDITOR",
    "GIT_SEQUENCE_EDITOR",
    "EDITOR",
    "VISUAL",
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "GIT_EXTERNAL_DIFF",
    "GIT_ASKPASS",
    "SSH_ASKPASS",
    "GIT_PROXY_COMMAND",
    "GIT_EXEC_PATH",
    "GIT_TEMPLATE_DIR",
    "GIT_ALLOW_PROTOCOL",
    "GIT_CONFIG",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_SYSTEM",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    // Where git finds the user's config (`$HOME/.gitconfig`).
    "HOME",
    "XDG_CONFIG_HOME",
    "LESS",
    "LESSOPEN",
    "LESSCLOSE",
    "LESSEDIT",
    "MANOPT",
    "BROWSER",
];
/// `GIT_CONFIG_KEY_<n>`/`GIT_CONFIG_VALUE_<n>` set config keys,
/// `GIT_TRACE*` names a file git writes, `GIT_TEST_*` are test hooks (some
/// run commands: `GIT_TEST_FSMONITOR`), `LESSKEY*` a file that sets less's
/// variables.
const CODE_VARIABLE_PREFIXES: &[&str] = &[
    "GIT_CONFIG_KEY_",
    "GIT_CONFIG_VALUE_",
    "GIT_TRACE",
    "GIT_TEST_",
    "LESSKEY",
];
/// Builtins whose operands name the variables they set.
const NAMING_BUILTINS: &[&str] = &[
    "declare",
    "typeset",
    "local",
    "export",
    "readonly",
    "integer",
    "float",
    "private",
    "read",
    "vared",
    "mapfile",
    "readarray",
    "getopts",
];
/// git `-c` keys that never name a command. Any other key may
/// (`core.pager`, `pager.log`, `alias.x`, `diff.x.textconv`, `include.path`).
const SAFE_GIT_KEYS: &[&str] = &[
    "user.name",
    "user.email",
    "core.quotepath",
    "core.autocrlf",
    "core.safecrlf",
    "core.abbrev",
    "init.defaultbranch",
    "pull.rebase",
    "pull.ff",
    "merge.ff",
    "rebase.autostash",
    "rebase.autosquash",
    "fetch.prune",
    "push.default",
    "push.autosetupremote",
    "diff.renames",
    "diff.noprefix",
    "diff.mnemonicprefix",
    "diff.algorithm",
    "status.showuntrackedfiles",
];
const SAFE_GIT_KEY_PREFIXES: &[&str] = &["color.", "advice."];
/// git options before the subcommand that take the next word as their value.
const GIT_VALUE_OPTIONS: &[&str] = &[
    "-C",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--super-prefix",
    "--list-cmds",
    "--attr-source",
];

fn is_code_variable(name: &str) -> bool {
    CODE_VARIABLES.contains(&name)
        || CODE_VARIABLE_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

fn is_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// Whether `word` names a code variable other than by reading it.
fn names_code_variable(word: &str) -> bool {
    // PowerShell's environment, in any case (`$env:pager = 'x'`).
    let powershell = ["$env:", "env:"].iter().find_map(|prefix| {
        word.get(..prefix.len())
            .filter(|head| head.eq_ignore_ascii_case(prefix))
            .map(|_| &word[prefix.len()..])
    });
    if let Some(rest) = powershell {
        let name_len = rest
            .find(|ch: char| !is_name_char(ch))
            .unwrap_or(rest.len());
        if is_code_variable(&rest[..name_len].to_ascii_uppercase()) {
            return true;
        }
    }
    // An attached option value (`printf -vPAGER`, `read -raEDITOR`).
    if let Some(cluster) = word.strip_prefix('-')
        && !cluster.starts_with('-')
        && CODE_VARIABLES.iter().any(|name| cluster.ends_with(name))
    {
        return true;
    }
    // The name as a token anywhere, unless it is read (`$PAGER`,
    // `${PAGER}`): `export PAGER`, `r=PAGER`, `${PAGER:=x}`.
    let mut start = None;
    for (at, ch) in word
        .char_indices()
        .chain(std::iter::once((word.len(), ' ')))
    {
        work::step();
        if is_name_char(ch) {
            start.get_or_insert(at);
            continue;
        }
        let Some(from) = start.take() else {
            continue;
        };
        if !is_code_variable(&word[from..at]) {
            continue;
        }
        let before = &word[..from];
        let read = before.ends_with('$') || (before.ends_with("${") && ch == '}');
        if !read {
            return true;
        }
    }
    false
}

/// Whether a simple command's words (as written and, in `words`, with
/// known variables substituted) set a code variable, or name a variable the
/// classifier cannot see in a builtin that sets it. `command` is the command
/// word's index and lowercase name.
pub(super) fn sets_code_variable(
    simple: &SimpleCommand,
    words: &[String],
    command: Option<&(usize, String)>,
) -> bool {
    let raw = &simple.words;
    if raw
        .iter()
        .chain(words)
        .any(|word| names_code_variable(word))
    {
        return true;
    }
    let dynamic = |word: &str| word.contains('$') || word.contains(shell::UNSEEN_OUTPUT);
    // An assignment before the command (`"$n=x" git`, `env "$n=x" git`)
    // whose name the classifier cannot see.
    let before = command.map_or(words.len(), |(index, _)| *index);
    if words[..before].iter().any(|word| {
        !word.starts_with('-') && word.split_once('=').is_some_and(|(name, _)| dynamic(name))
    }) {
        return true;
    }
    let Some((index, name)) = command else {
        return false;
    };
    let name = name.as_str();
    let start = index + 1;
    // Windows names are in any case: `set pager=x` (cmd), `setx pager x`.
    if matches!(name, "set" | "setx") {
        return words[start..].iter().any(|word| names_in_any_case(word));
    }
    let redirect = |at: usize| {
        simple.stdin_from == Some(at)
            || simple.here_string == Some(at)
            || simple.writes_to.contains(&at)
    };
    // A name the classifier cannot see, or one in any case when it came
    // from a variable: a value given the uppercase attribute (`declare -u
    // n=pager; printf -v "$n"`) names `PAGER`.
    let judged = |at: usize, name: &str| {
        !redirect(at)
            && (dynamic(name) || (raw.get(at) != words.get(at) && names_in_any_case(name)))
    };
    // Options whose value is not a name (`read -p PROMPT`), and the one
    // whose value is (`read -a NAME`).
    let (value_flags, name_flag) = match name {
        "printf" | "print" => ("", Some('v')),
        "read" => ("dinNptu", Some('a')),
        "mapfile" | "readarray" => ("dnOsuCc", None),
        "vared" => ("prMmit", None),
        _ if NAMING_BUILTINS.contains(&name) => ("", None),
        _ => return false,
    };
    // A nameref's target (`declare -n r="$x"`) is a name too.
    let nameref = matches!(name, "declare" | "typeset" | "local")
        && words[start..]
            .iter()
            .any(|word| word.starts_with('-') && !word.starts_with("--") && word.contains('n'));
    let printf = name_flag == Some('v');
    let mut skip = false;
    let mut name_next = false;
    // `getopts OPTSTRING NAME`: the first operand is not a name.
    let mut optstring = name == "getopts";
    for (at, word) in words.iter().enumerate().skip(start) {
        work::step();
        if std::mem::take(&mut skip) {
            continue;
        }
        if std::mem::take(&mut name_next) {
            if judged(at, word) {
                return true;
            }
            continue;
        }
        if let Some(cluster) = word.strip_prefix('-')
            && !cluster.is_empty()
        {
            let flag = cluster
                .char_indices()
                .find(|(_, flag)| value_flags.contains(*flag) || Some(*flag) == name_flag);
            if let Some((flag_at, flag)) = flag {
                let attached = &cluster[flag_at + flag.len_utf8()..];
                if Some(flag) == name_flag && !attached.is_empty() {
                    if judged(at, attached) {
                        return true;
                    }
                } else if Some(flag) == name_flag {
                    name_next = true;
                } else {
                    skip = attached.is_empty();
                }
            }
            continue;
        }
        if printf || std::mem::take(&mut optstring) {
            continue;
        }
        let name_part = if nameref {
            word.as_str()
        } else {
            word.split_once('=').map_or(word.as_str(), |(name, _)| name)
        };
        if judged(at, name_part) {
            return true;
        }
    }
    false
}

/// Whether a name operand, or a piece of it between `=` (a nameref's
/// target), is a code variable in any case.
fn names_in_any_case(operand: &str) -> bool {
    operand
        .split('=')
        .any(|piece| is_code_variable(&piece.to_ascii_uppercase()))
}

/// Whether git's options before its subcommand (`args`, as written) set a
/// config key that may name a command (`-c core.pager=x`, `--config-env`)
/// or point git at other programs (`--exec-path=dir`).
pub(super) fn git_runs_code(args: &[String]) -> bool {
    let unsafe_key = |setting: &str| {
        let key = setting
            .split_once('=')
            .map_or(setting, |(key, _)| key)
            .to_ascii_lowercase();
        !(SAFE_GIT_KEYS.contains(&key.as_str())
            || SAFE_GIT_KEY_PREFIXES
                .iter()
                .any(|prefix| key.starts_with(prefix)))
    };
    let mut index = 0;
    while let Some(word) = args.get(index) {
        work::step();
        let next = args.get(index + 1).map(String::as_str);
        if word == "-c" || word == "--config-env" {
            if next.is_none_or(unsafe_key) {
                return true;
            }
            index += 2;
            continue;
        }
        if let Some(setting) = word.strip_prefix("--config-env=") {
            if unsafe_key(setting) {
                return true;
            }
        } else if word.starts_with("--exec-path=")
            || (word.starts_with("-c") && !word.starts_with("--") && unsafe_key(&word[2..]))
        {
            return true;
        } else if !word.starts_with('-') || shell::basename(word) == "git" {
            // The subcommand: its own options follow (`commit -c HEAD`). A
            // later `git` is checked on its own (#1102: each word is
            // scanned once).
            return false;
        }
        index += 1 + usize::from(GIT_VALUE_OPTIONS.contains(&word.as_str()));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::SimpleCommand;
    use super::git_runs_code;
    use super::sets_code_variable;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_string()).collect()
    }

    /// `raw` as written, `words` with variables substituted.
    fn sets_from(raw: &[&str], words: &[&str]) -> bool {
        let simple = SimpleCommand {
            words: strings(raw),
            ..SimpleCommand::default()
        };
        let words = strings(words);
        let command = super::super::indirect::command_word(&words);
        sets_code_variable(&simple, &words, command.as_ref())
    }

    fn sets(words: &[&str]) -> bool {
        sets_from(words, words)
    }

    #[test]
    fn code_variable_settings_are_found() {
        for words in [
            &["GIT_PAGER=touch x", "git", "log"][..],
            &["env", "PAGER=x", "git", "log"],
            &["export", "GIT_EDITOR=x"],
            &["export", "PAGER"],
            &["printf", "-v", "PAGER", "%s", "x"],
            &["printf", "-vGIT_PAGER", "%s", "x"],
            &["read", "-r", "EDITOR"],
            &["read", "-raEDITOR"],
            &["declare", "-n", "r=PAGER"],
            &[":", "${PAGER:=x}"],
            &["GIT_CONFIG_KEY_0=core.pager"],
            &["PAGER+=x"],
            &["PAGER[0]=x"],
            &["$env:git_pager='x'"],
            &["set", "pager=x"],
            &["for", "PAGER", "in", "x"],
            &["printf", "-v", "$name", "%s", "x"],
            &["declare", "$name=x"],
            &["declare", "-n", "r=$name"],
            &["read", "$name"],
            &["GIT_TRACE=/tmp/t", "git", "status"],
            &["HOME=/tmp/h", "git", "log"],
            &["LESSKEYIN=/tmp/k", "less", "f"],
            &["GIT_TEST_FSMONITOR=x", "git", "status"],
            &["env", "$n=x", "git", "diff"],
            &["\u{1}unseen-output\u{1}=x", "git", "diff"],
        ] {
            assert!(sets(words), "{words:?}");
        }
        for words in [
            &["git", "log", "--oneline"][..],
            &["echo", "$PAGER"],
            &["echo", "${GIT_PAGER}"],
            &["printf", "%s", "$x"],
            &["declare", "x=$y"],
            &["export", "PAGERS=x"],
            &["declare", "-u", "name=value"],
            &["cd", "$HOME/src"],
            &["MY_EDITOR=x"],
            &["X=$(date)", "git", "log"],
            &["sudo", "-u", "$user", "git", "log"],
            &["git", "commit", "-m", "fix editor crash"],
        ] {
            assert!(!sets(words), "{words:?}");
        }
    }

    #[test]
    fn uppercased_names_from_variables_are_found() {
        assert!(sets_from(
            &["printf", "-v", "$n", "%s", "x"],
            &["printf", "-v", "pager", "%s", "x"]
        ));
        assert!(sets_from(
            &["declare", "-n", "r=$t"],
            &["declare", "-n", "r=git_pager"]
        ));
        assert!(sets_from(&["read", "-a", "$n"], &["read", "-a", "editor"]));
        // A literal lowercase name is another variable.
        assert!(!sets(&["local", "home=x", "editor"]));
        assert!(!sets(&["printf", "-v", "pager", "%s", "x"]));
    }

    #[test]
    fn options_that_are_not_names_are_skipped() {
        for words in [
            &["read", "-p", "$prompt", "x"][..],
            &["read", "-rt$timeout", "x"],
            &["mapfile", "-u", "$fd", "lines"],
            &["getopts", "$optstring", "opt"],
            &["printf", "-v", "x", "%s", "$y"],
        ] {
            assert!(!sets(words), "{words:?}");
        }
        for words in [
            &["read", "-p", "x", "$name"][..],
            &["read", "-ra$name"],
            &["getopts", "ab", "$name"],
            &["setx", "pager", "x"],
        ] {
            assert!(sets(words), "{words:?}");
        }
    }

    #[test]
    fn git_overrides_that_may_run_code_are_found() {
        for args in [
            &["-c", "core.pager=touch x", "log"][..],
            &["-c", "alias.x=!sh", "x"],
            &["-c", "Core.Pager=x", "log"],
            &["-C", "dir", "-c", "pager.log=x", "log"],
            &["--config-env=core.editor=E", "commit"],
            &["--config-env", "diff.x.textconv=T", "diff"],
            &["--exec-path=/tmp/x", "log"],
            &["-c"],
        ] {
            assert!(git_runs_code(&strings(args)), "{args:?}");
        }
        for args in [
            &["log", "--oneline"][..],
            &["-C", "dir", "status"],
            &["-c", "user.name=x", "-c", "color.ui=always", "commit"],
            &["commit", "-c", "HEAD"],
            &["--exec-path"],
        ] {
            assert!(!git_runs_code(&strings(args)), "{args:?}");
        }
    }
}
