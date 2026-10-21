//! #1141: commands that write the file an output option names
//! (`git log --output=FILE`, `git archive -o FILE`, `sort -o FILE`). Such a
//! command is a writer like `tee`, and the option's value is a file it
//! writes.

use super::work;

/// Commands with an output-file option: the long option (git and sort
/// accept any unambiguous prefix of it, `--out=FILE`) and its short letter.
/// git's `--output-directory` (`format-patch`) starts with `--output`, so a
/// prefix of it covers both; `-o` means output to `archive` and
/// `format-patch` and is taken as output for every git subcommand.
const OUTPUT_OPTIONS: &[(&str, &str, char)] = &[
    ("git", "--output-directory", 'o'),
    ("sort", "--output", 'o'),
];

/// git options that name a file `fast-export`/`fast-import` writes.
const GIT_MARK_OPTIONS: &[&str] = &["--export-marks", "--export-pack-edges"];
/// A short letter no option word holds.
const NO_SHORT: char = '\0';

/// Shortest long-option prefix taken as the output option (`--o`).
const MIN_PREFIX: usize = 3;

/// For a command that has an output-file option and uses it, the files it
/// names (empty when the value is missing); `None` otherwise. `name` is the
/// command's lowercase basename, `args` the words after it as written.
pub(super) fn output_files(name: &str, args: &[String]) -> Option<Vec<String>> {
    let (_, long, short) = OUTPUT_OPTIONS
        .iter()
        .find(|(command, _, _)| *command == name)?;
    let has = |word: &str| args.iter().any(|arg| arg == word);
    // `git config --file FILE key value` writes FILE; `git bundle create
    // FILE` writes its operand (a writer's operands are written).
    let config_file = name == "git"
        && args
            .iter()
            .position(|arg| arg == "config")
            .is_some_and(|at| !git_config_reads(&args[at + 1..]));
    let mut found = name == "git" && has("bundle") && has("create");
    let mut files = Vec::new();
    if config_file {
        found |= option_values(args, "--file", 'f', &mut files);
    }
    // `git credential-store --file FILE store` appends to FILE.
    if name == "git" && has("credential-store") {
        found |= option_values(args, "--file", NO_SHORT, &mut files);
    }
    found |= option_values(args, long, *short, &mut files);
    if name == "git" {
        // `fast-export`/`fast-import` write their marks and pack edges.
        for long in GIT_MARK_OPTIONS {
            work::step();
            found |= option_values(args, long, NO_SHORT, &mut files);
        }
    }
    found.then_some(files)
}

/// git config options that take the next word as their value (`-zf FILE`
/// too).
const GIT_CONFIG_VALUES: &[&str] = &[
    "-f",
    "--file",
    "--blob",
    "--type",
    "--default",
    "--comment",
    "--value",
    "--url",
];
/// git config options that take no value (others may take the next word).
const GIT_CONFIG_FLAGS: &[&str] = &[
    "--global",
    "--system",
    "--local",
    "--worktree",
    "--show-origin",
    "--show-scope",
    "--null",
    "--name-only",
    "--includes",
    "--no-includes",
    "--bool",
    "--int",
    "--bool-or-int",
    "--path",
    "--expiry-date",
    "--fixed-value",
    "--all",
    "--no-type",
];
/// git config options that make it read, not write. As on main, only
/// these: `--get-color`, `--get-colorbool` and `--get-urlmatch` are not
/// counted, so `git config --global --get-color k` asks (review round 8).
const GIT_CONFIG_READS: &[&str] = &["--get", "--get-all", "--get-regexp", "--list", "-l"];

/// Whether `git config` with these words (those after `config`) only reads:
/// its first word is the `get` or `list` subcommand, or a reading option
/// comes before the first word that is not an option. git stops reading
/// options there, so every later word is a key, value or pattern (`k v
/// --get`, `--comment --get`, `-- --get` write). Read conservatively: a
/// word that may be a value is one. Stops at the next `git` (#1102: each
/// word is scanned once per line).
pub(super) fn git_config_reads(words: &[String]) -> bool {
    if words
        .first()
        .is_some_and(|word| matches!(word.as_str(), "get" | "list"))
    {
        return true;
    }
    // The previous word was an option that takes a value, or an unknown
    // one that may.
    let mut value_next = false;
    let mut maybe_value_next = false;
    for word in words {
        work::step();
        if std::mem::take(&mut value_next) {
            continue;
        }
        let option = word.starts_with('-');
        if std::mem::take(&mut maybe_value_next) && option {
            // A value (`--comment -l`), not an option.
            continue;
        }
        // A word that is not an option may be the key (after an unknown
        // option too), which ends the options.
        if word == "--" || !option || super::shell::basename(word) == "git" {
            return false;
        }
        if GIT_CONFIG_READS.contains(&word.as_str()) {
            return true;
        }
        if word.contains('=') || GIT_CONFIG_FLAGS.contains(&word.as_str()) {
            continue;
        }
        if GIT_CONFIG_VALUES.contains(&word.as_str())
            || (!word.starts_with("--") && word.ends_with('f'))
        {
            value_next = true;
        } else {
            maybe_value_next = true;
        }
    }
    false
}

/// Push the values of `long` (or a prefix of it) and `short` in `args` to
/// `files`; whether the option is used.
fn option_values(args: &[String], long: &str, short: char, files: &mut Vec<String>) -> bool {
    let mut found = false;
    for (index, word) in args.iter().enumerate() {
        work::step();
        let next = || args.get(index + 1).cloned();
        if let Some(option) = word.strip_prefix("--") {
            let (option, value) = match option.split_once('=') {
                Some((option, value)) => (option, Some(value.to_string())),
                None => (option, None),
            };
            let option = format!("--{option}");
            if option.len() >= MIN_PREFIX && long.starts_with(option.as_str()) {
                found = true;
                files.extend(value.or_else(next));
            }
        } else if let Some(cluster) = word.strip_prefix('-')
            && let Some(at) = cluster.find(short)
        {
            // A group of short options (`-vo FILE`, `-oFILE`): the rest of
            // the group after the letter is its value, else the next word.
            found = true;
            let attached = &cluster[at + short.len_utf8()..];
            if attached.is_empty() {
                files.extend(next());
            } else {
                files.push(attached.to_string());
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::output_files;
    use pretty_assertions::assert_eq;

    fn files(name: &str, args: &[&str]) -> Option<Vec<String>> {
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
        output_files(name, &args)
    }

    #[test]
    fn output_options_name_written_files() {
        let some = |files: &[&str]| Some(files.iter().map(|file| (*file).to_string()).collect());
        for (name, args, expected) in [
            (
                "git",
                &["log", "--output=~/.zshrc"][..],
                some(&["~/.zshrc"]),
            ),
            ("git", &["log", "--output", "f"], some(&["f"])),
            ("git", &["diff", "--outp=f"], some(&["f"])),
            ("git", &["archive", "-o", "f", "HEAD"], some(&["f"])),
            ("git", &["archive", "-of"], some(&["f"])),
            ("git", &["archive", "-vof"], some(&["f"])),
            (
                "git",
                &["format-patch", "--output-directory", "d"],
                some(&["d"]),
            ),
            ("git", &["log", "--output"], some(&[])),
            (
                "git",
                &["config", "--file", "~/.zshrc", "a.b", "x"],
                some(&["~/.zshrc"]),
            ),
            (
                "git",
                &["config", "-f~/.zshrc", "a.b", "x"],
                some(&["~/.zshrc"]),
            ),
            ("git", &["bundle", "create", "~/.zshrc", "HEAD"], some(&[])),
            (
                "git",
                &["fast-export", "--export-marks=~/.zshrc", "HEAD"],
                some(&["~/.zshrc"]),
            ),
            (
                "git",
                &["credential-store", "--file", "~/.zshrc", "store"],
                some(&["~/.zshrc"]),
            ),
            ("sort", &["-o", "f", "in"], some(&["f"])),
            ("sort", &["-ro~/.zshrc"], some(&["~/.zshrc"])),
            ("sort", &["--output=f"], some(&["f"])),
            ("git", &["log", "--oneline", "-n", "5"], None),
            ("git", &["log", "--output-indicator-new=+"], None),
            ("sort", &["--other=f"], None),
            ("git", &["commit", "--file=msg"], None),
            (
                "git",
                &["config", "-f", "~/.gitconfig", "--get", "a.b"],
                None,
            ),
            ("cat", &["-o", "f"], None),
        ] {
            assert_eq!(files(name, args), expected, "{name} {args:?}");
        }
    }
}
