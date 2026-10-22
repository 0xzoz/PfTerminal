//! #1323: the words of a command that are search patterns (`rg ".env"
//! logs/`), not paths the command opens. A search that exits 0 can print old
//! denial lines naming its pattern; such a word explains nothing.
//!
//! Text-based and conservative: only `grep`, `egrep`, `fgrep`, `rg` and
//! `git grep` are recognised, and a word is a pattern only when the parse is
//! sure (an option it does not know is taken as a flag, which can only mark
//! an option's value instead of the pattern). Explanation only.

use std::collections::HashSet;

/// Words that end a simple command.
const SEPARATORS: [&str; 9] = ["|", "||", "&&", ";", "&", "|&", "(", ")", "!"];

/// Words after which the next word is a program.
const PROGRAM_BEFORE: [&str; 10] = [
    "then", "do", "else", "{", "sudo", "env", "time", "nohup", "command", "exec",
];

#[derive(Clone, Copy, PartialEq)]
enum Search {
    /// `grep`, `egrep`, `fgrep`, `git grep`.
    Grep,
    /// `rg`.
    Ripgrep,
}

/// What an option takes as its value.
#[derive(Clone, Copy, PartialEq)]
enum Value {
    /// Nothing: a flag (also an option this does not know).
    Nothing,
    /// The rest of the word, or the next word.
    Any,
    /// A number: the next word only when it is one (BSD `grep -C` takes an
    /// optional count).
    Number,
}

impl Search {
    /// What a short option takes.
    fn short_value(self, option: char) -> Value {
        match (self, option) {
            (_, 'A' | 'B' | 'C' | 'm') | (Self::Ripgrep, 'j' | 'M') => Value::Number,
            (_, 'e' | 'f')
            | (Self::Grep, 'D' | 'd')
            | (Self::Ripgrep, 'E' | 'g' | 'r' | 't' | 'T') => Value::Any,
            _ => Value::Nothing,
        }
    }

    /// What a long option, without `=`, takes from the next word.
    fn long_value(self, option: &str) -> Value {
        const NUMBER: [&str; 7] = [
            "--context",
            "--after-context",
            "--before-context",
            "--max-count",
            "--max-depth",
            "--threads",
            "--max-columns",
        ];
        const GREP: [&str; 9] = [
            "--file",
            "--include",
            "--exclude",
            "--exclude-dir",
            "--exclude-from",
            "--label",
            "--devices",
            "--directories",
            "--binary-files",
        ];
        const RIPGREP: [&str; 13] = [
            "--file",
            "--ignore-file",
            "--color",
            "--glob",
            "--iglob",
            "--type",
            "--type-not",
            "--type-add",
            "--replace",
            "--encoding",
            "--max-filesize",
            "--sort",
            "--sortr",
        ];
        let any = match self {
            Self::Grep => GREP.contains(&option),
            Self::Ripgrep => RIPGREP.contains(&option),
        };
        if NUMBER.contains(&option) {
            Value::Number
        } else if any {
            Value::Any
        } else {
            Value::Nothing
        }
    }

    /// Modes in which no word is a pattern (`rg --files` lists paths).
    fn no_pattern(self, option: &str) -> bool {
        self == Self::Ripgrep && matches!(option, "--files" | "--type-list")
    }
}

/// Whether the word after an option taking `value` is that value.
fn takes_next(value: Value, next: Option<&String>) -> bool {
    match value {
        Value::Nothing => false,
        Value::Any => next.is_some(),
        Value::Number => {
            next.is_some_and(|word| !word.is_empty() && word.chars().all(|c| c.is_ascii_digit()))
        }
    }
}

/// The indices into `words` (a command split into words) of the search
/// patterns: a pattern word (`rg PATTERN path`), the value of `-e`
/// (`grep -e PATTERN path`), or `--regexp=PATTERN`.
pub(super) fn search_pattern_indices(words: &[String]) -> HashSet<usize> {
    let mut patterns = HashSet::new();
    let mut index = 0;
    while index < words.len() {
        let start = program_position(words, index)
            .then(|| search_program(words, index))
            .flatten();
        match start {
            Some((search, first)) => {
                index = mark_patterns(words, search, first, &mut patterns);
            }
            None => index += 1,
        }
    }
    patterns
}

/// Whether `words[index]` is where a program is named.
fn program_position(words: &[String], mut index: usize) -> bool {
    // `sudo grep`, `then grep`: where the word before is itself a program
    // or a keyword (not `cat do grep .env`).
    while index > 0 && PROGRAM_BEFORE.contains(&words[index - 1].as_str()) {
        index -= 1;
    }
    let Some(previous) = index.checked_sub(1).map(|index| words[index].as_str()) else {
        return true;
    };
    if SEPARATORS.contains(&previous) || previous.ends_with([';', '|', '&']) {
        return true;
    }
    // `bash -c 'grep …'`, `sh -lc …`: the script's first word.
    let is_shell_option =
        previous.starts_with('-') && !previous.starts_with("--") && previous.contains('c');
    is_shell_option
        && index
            .checked_sub(2)
            .is_some_and(|index| super::posix_shell(&words[index]))
}

/// The search `words[index]` starts, and the index of its first argument.
fn search_program(words: &[String], index: usize) -> Option<(Search, usize)> {
    let word = words[index].trim_start_matches('(');
    let name = word.rsplit('/').next().unwrap_or(word);
    match name {
        "grep" | "egrep" | "fgrep" => Some((Search::Grep, index + 1)),
        "rg" => Some((Search::Ripgrep, index + 1)),
        "git" if words.get(index + 1).is_some_and(|word| word == "grep") => {
            Some((Search::Grep, index + 2))
        }
        _ => None,
    }
}

/// Marks the patterns of the search whose arguments start at `first`, and
/// returns the index of the word after its last argument.
fn mark_patterns(
    words: &[String],
    search: Search,
    first: usize,
    patterns: &mut HashSet<usize>,
) -> usize {
    let mut operands = Vec::new();
    let mut given = Vec::new();
    // `-e` or `-f` gives the patterns, or a mode takes none: no operand is
    // a pattern then.
    let mut no_operand_pattern = false;
    let mut options_end = false;
    let mut index = first;
    while index < words.len() {
        let word = words[index].as_str();
        if SEPARATORS.contains(&word) {
            break;
        }
        let last = word.ends_with([';', '|', '&']);
        if !options_end && word == "--" {
            options_end = true;
        } else if !options_end && word.starts_with("--") {
            let (option, value) = match word.split_once('=') {
                Some((option, _)) => (option, true),
                None => (word, false),
            };
            // GNU getopt takes an unambiguous abbreviation (`--reg`).
            let option = if search == Search::Grep
                && option.len() >= "--reg".len()
                && "--regexp".starts_with(option)
            {
                "--regexp"
            } else {
                option
            };
            match option {
                "--regexp" => {
                    no_operand_pattern = true;
                    if value {
                        given.push(index);
                    } else if !last {
                        index += 1;
                        given.push(index);
                    }
                }
                "--file" => no_operand_pattern = true,
                _ => no_operand_pattern |= search.no_pattern(option),
            }
            if !value
                && !last
                && option != "--regexp"
                && takes_next(search.long_value(option), words.get(index + 1))
            {
                index += 1;
            }
        } else if !options_end && word.starts_with('-') && word.len() > 1 {
            // A cluster of short options (`-inA3`, `-ie`, `-e.env`).
            let options: Vec<char> = word.chars().skip(1).collect();
            for (position, &option) in options.iter().enumerate() {
                if option == 'e' || option == 'f' {
                    no_operand_pattern = true;
                }
                let value = search.short_value(option);
                if value != Value::Nothing {
                    // The rest of the word, or the next word.
                    if position + 1 == options.len()
                        && !last
                        && takes_next(value, words.get(index + 1))
                    {
                        index += 1;
                        if option == 'e' {
                            given.push(index);
                        }
                    }
                    break;
                }
            }
        } else if let Some(bare) = redirection(word) {
            // `2>/dev/null`; `> out`, `2> err`, `&>> log` take the next word.
            if bare && !last {
                index += 1;
            }
        } else {
            operands.push(index);
        }
        index += 1;
        if last {
            break;
        }
    }
    // A pattern that runs a command (`grep "$(cat ~/.ssh/id_rsa)" notes`)
    // names the paths that command opens: kept.
    let plain = |&index: &usize| {
        words
            .get(index)
            .is_some_and(|word| !word.contains("$(") && !word.contains('`'))
    };
    patterns.extend(given.into_iter().filter(plain));
    if !no_operand_pattern
        && let Some(&pattern) = operands.first()
        && plain(&pattern)
    {
        patterns.insert(pattern);
    }
    index
}

/// Whether `word` redirects (`>out`, `2>/dev/null`, `<in`, `&>log`), and
/// if so whether its target is the next word (`>`, `2>>`, `&>`).
fn redirection(word: &str) -> Option<bool> {
    let rest = word
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim_start_matches('&');
    rest.starts_with(['<', '>'])
        .then(|| rest.chars().all(|c| matches!(c, '<' | '>')))
}

#[cfg(test)]
#[path = "search_patterns_tests.rs"]
mod tests;
