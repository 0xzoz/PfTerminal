//! #1159: local Git writes outside the shared command-free key allowlist.
//! The existing read detector rules out reads; uncertain options must not
//! turn a possible persistent code setting into a silent action.

use super::basename;
use super::code_variables;
use super::git_helpers;
use super::outputs;
use super::work;

enum ConfigArgs<'a> {
    Found(&'a [String]),
    NotConfig,
    Uncertain,
}

/// Find `config`, sharing Git's global value-option boundaries. Only plain
/// Git lets the caller restart with the same suffix. A dashed-helper-looking
/// path or option value cannot safely end this check without asking.
fn config_args(args: &[String]) -> ConfigArgs<'_> {
    let mut at = 0;
    while let Some(word) = args.get(at) {
        work::step();
        let takes_value = code_variables::GIT_VALUE_OPTIONS.contains(&word.as_str())
            || matches!(word.as_str(), "-c" | "--config-env");
        let boundary = if takes_value {
            args.get(at + 1)
        } else {
            Some(word)
        };
        if let Some(boundary) = boundary {
            let git_name = git_helpers::git_name(boundary);
            if basename(boundary).eq_ignore_ascii_case("git") || git_name == Some(None) {
                return ConfigArgs::NotConfig;
            }
            if git_name.is_some() {
                return ConfigArgs::Uncertain;
            }
        }
        if word == "config" {
            return ConfigArgs::Found(&args[at + 1..]);
        }
        if takes_value {
            at += 2;
        } else if word.starts_with('-') {
            at += 1;
        } else {
            return ConfigArgs::NotConfig;
        }
    }
    ConfigArgs::NotConfig
}

/// Local config writes, sharing work across all Git words in a command.
/// A Git-looking word can be an option's value, so scans from different
/// Git words may overlap even when each scan respects option boundaries.
pub(super) struct Writes<'a> {
    words: &'a [String],
    seen: Option<FollowingWrites>,
}

struct FollowingWrites {
    reads: Vec<bool>,
    /// A suffix's result for each combination of the two parser flags.
    writes: Vec<[Option<bool>; 4]>,
}

impl<'a> Writes<'a> {
    pub(super) fn new(words: &'a [String]) -> Self {
        Self { words, seen: None }
    }

    /// Whether a possible local write sets a key outside the same allowlist
    /// used for `git -c`. Values do not make an unsafe key safe: even `false`
    /// or `rebase` asks after untrusted content. Section renames can install
    /// previously harmless keys in a program-running family and also ask.
    pub(super) fn writes_code_setting(&mut self, from: usize, subcommand: Option<&str>) -> bool {
        let words = self.words;
        let args = &words[from..];
        let args = match subcommand {
            Some("config") => args,
            Some(_) => return false,
            None => match config_args(args) {
                ConfigArgs::Found(args) => args,
                ConfigArgs::NotConfig => return false,
                ConfigArgs::Uncertain => return true,
            },
        };
        let mut at = words.len() - args.len();
        let seen = self.seen.get_or_insert_with(|| FollowingWrites {
            reads: outputs::git_config_reads_each(words),
            writes: vec![[None; 4]; words.len()],
        });
        if seen.reads[at] {
            return false;
        }
        let mut subcommand_position = true;
        let mut options = true;
        let mut visited = Vec::new();
        let writes = loop {
            work::step();
            let Some(word) = words.get(at) else {
                break false;
            };
            let state = usize::from(options) | (usize::from(subcommand_position) << 1);
            if let Some(writes) = seen.writes[at][state] {
                break writes;
            }
            visited.push((at, state));
            let first = std::mem::take(&mut subcommand_position);
            if options && word == "--" {
                options = false;
                at += 1;
                continue;
            }
            if options && word.starts_with('-') {
                let option = word.split_once('=').map_or(word.as_str(), |(name, _)| name);
                if matches!(
                    word.as_str(),
                    "--unset" | "--unset-all" | "--remove-section" | "--help" | "-h"
                ) {
                    break false;
                }
                if word == "--rename-section" {
                    break words.get(at + 2).is_some();
                }
                if outputs::GIT_CONFIG_VALUES.contains(&option) || option == "-t" {
                    at += if word.contains('=') { 1 } else { 2 };
                } else if outputs::GIT_CONFIG_FLAGS.contains(&word.as_str())
                    || matches!(
                        word.as_str(),
                        "--add" | "--replace-all" | "--append" | "--bool-or-str"
                    )
                {
                    at += 1;
                } else if let Some(short) = word.strip_prefix('-')
                    && !short.starts_with('-')
                    && let Some((offset, flag)) =
                        short.char_indices().find(|(_, ch)| matches!(ch, 'f' | 't'))
                    && short[..offset].chars().all(|ch| ch == 'z')
                {
                    // A short option group (`-zf FILE`) or attached value.
                    at += 1 + usize::from(offset + flag.len_utf8() == short.len());
                } else {
                    // An abbreviation can rename a section or take the next
                    // word as its value. The read check did not prove a read;
                    // ask instead of guessing where a write's key starts.
                    break true;
                }
                continue;
            }
            // At the key position, a Git-looking word is still a key. The
            // suffix memo keeps overlapping invocations linear.
            // After any option, Git uses legacy key/value syntax. Only the
            // first argument can select a subcommand such as `unset`.
            if options && first {
                match word.as_str() {
                    "unset" | "remove-section" => break false,
                    "rename-section" => break words.get(at + 2).is_some(),
                    "set" => {
                        at += 1;
                        continue;
                    }
                    _ => {}
                }
            }
            let safe = |key: &str| code_variables::safe_git_key(&key.to_ascii_lowercase());
            break words.get(at + 1).is_some() && !safe(word);
        };
        // Each word/state is scanned at most once across all invocations.
        for (at, state) in visited {
            work::step();
            seen.writes[at][state] = Some(writes);
        }
        writes
    }
}

/// The shared resolver deliberately skips wrapper options conservatively.
/// For this additional detector only, exclude wrappers that inspect names or
/// print their own help instead of running the resolved Git command. The
/// prefix ends at that resolved command, so other words here are assignments,
/// shell keywords or wrappers already recognized by `command_word`.
/// Consume their value options too: `env -u command git ...` does not invoke
/// the `command` builtin. Existing classifier checks remain unchanged.
pub(super) fn inspection_only_prefix(prefix: &[String]) -> bool {
    let mut at = 0;
    while let Some(word) = prefix.get(at) {
        work::step();
        let name = basename(word).to_lowercase();
        at += 1;
        if word.contains('=')
            || matches!(
                name.as_str(),
                "!" | "if" | "then" | "elif" | "else" | "do" | "while" | "until"
            )
        {
            continue;
        }
        // Operand boundaries are the same as the unchanged shared resolver.
        let (value_options, mut operands) = super::indirect::WRAPPERS
            .iter()
            .find(|(wrapper, _, _)| *wrapper == name)
            .map_or((&[][..], 0), |(_, options, operands)| (*options, *operands));
        let mut options = true;
        while let Some(option) = prefix.get(at) {
            work::step();
            if option.starts_with('-') {
                if options
                    && ((name == "command"
                        && !option.starts_with("--")
                        && option.chars().skip(1).any(|ch| matches!(ch, 'v' | 'V')))
                        || (name == "env" && matches!(option.as_str(), "--help" | "--version")))
                {
                    return true;
                }
                if option == "--" {
                    options = false;
                }
                if name == "taskset" && matches!(option.as_str(), "-c" | "--cpu-list") {
                    operands = 0;
                }
                at += 1 + usize::from(value_options.contains(&option.as_str()));
            } else if option.contains('=') {
                at += 1;
            } else {
                break;
            }
        }
        at += operands;
    }
    false
}
