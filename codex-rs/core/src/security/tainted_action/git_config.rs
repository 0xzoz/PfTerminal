//! #1159: local Git writes outside the shared command-free key allowlist.
//! The existing read detector rules out reads; uncertain options must not
//! turn a possible persistent code setting into a silent action.

use super::basename;
use super::code_variables;
use super::git_helpers;
use super::outputs;
use super::work;

fn is_git(word: &str) -> bool {
    basename(word).eq_ignore_ascii_case("git") || git_helpers::git_name(word).is_some()
}

/// Find `config`, sharing Git's global value-option boundaries. Unknown
/// options are not a reason to stop checking. Stop at the next Git word so
/// the caller's per-word scan never rescans a later invocation (#1102).
fn config_args(args: &[String]) -> Option<&[String]> {
    let mut at = 0;
    while let Some(word) = args.get(at) {
        work::step();
        if is_git(word) {
            return None;
        }
        if word == "config" {
            return Some(&args[at + 1..]);
        }
        if code_variables::GIT_VALUE_OPTIONS.contains(&word.as_str())
            || matches!(word.as_str(), "-c" | "--config-env")
        {
            if args.get(at + 1).is_some_and(|value| is_git(value)) {
                return None;
            }
            at += 2;
        } else if word.starts_with('-') {
            at += 1;
        } else {
            return None;
        }
    }
    None
}

/// Whether a possible local write sets a key outside the same allowlist
/// used for `git -c`. Values do not make an unsafe key safe: even `false`
/// or `rebase` asks after untrusted content. Section renames can install
/// previously harmless keys in a program-running family and also ask.
pub(super) fn writes_code_setting(args: &[String], subcommand: Option<&str>) -> bool {
    let args = match subcommand {
        Some("config") => args,
        Some(_) => return false,
        None => match config_args(args) {
            Some(args) => args,
            None => return false,
        },
    };
    if outputs::git_config_reads(args) {
        return false;
    }
    let mut at = 0;
    let mut action_seen = false;
    let mut options = true;
    let mut maybe_value = false;
    while let Some(word) = args.get(at) {
        work::step();
        if is_git(word) {
            return false;
        }
        if options && word == "--" {
            options = false;
            at += 1;
            continue;
        }
        if options && word.starts_with('-') {
            let option = word.split_once('=').map_or(word.as_str(), |(name, _)| name);
            if matches!(
                option,
                "--unset" | "--unset-all" | "--remove-section" | "--help" | "-h"
            ) {
                return false;
            }
            if option == "--rename-section" {
                return args.get(at + 2).is_some();
            }
            if outputs::GIT_CONFIG_VALUES.contains(&option) || option == "-t" {
                at += if word.contains('=') { 1 } else { 2 };
            } else if outputs::GIT_CONFIG_FLAGS.contains(&option)
                || matches!(
                    option,
                    "-z" | "--add" | "--replace-all" | "--append" | "--bool-or-str"
                )
                || word.contains('=')
            {
                at += 1;
            } else if !word.starts_with("--")
                && let Some((offset, flag)) =
                    word.char_indices().find(|(_, ch)| matches!(ch, 'f' | 't'))
            {
                // A short option group (`-zf FILE`) or attached value.
                at += 1 + usize::from(offset + flag.len_utf8() == word.len());
            } else {
                // Could be an abbreviated flag or a value-taking option.
                // Judge both possible key positions; never silently give up.
                maybe_value = true;
                at += 1;
            }
            continue;
        }
        if options && !action_seen {
            match word.as_str() {
                "unset" | "remove-section" => return false,
                "rename-section" => return args.get(at + 2).is_some(),
                "set" => {
                    action_seen = true;
                    at += 1;
                    continue;
                }
                _ => {}
            }
        }
        let safe = |key: &str| code_variables::safe_git_key(&key.to_ascii_lowercase());
        return args.get(at + 1).is_some_and(|next| {
            !safe(word) || (maybe_value && args.get(at + 2).is_some() && !safe(next))
        });
    }
    false
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
