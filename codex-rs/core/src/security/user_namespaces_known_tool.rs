//! D48 (c) (Travis, 2026-10-20, security sign-off, #891): after untrusted
//! content under Moderate on Linux, the one-run user-namespace allowance is
//! offered only for a command whose program is a known tool that needs one.
//!
//! An allow-list, decided on the parsed command, never by searching its text:
//! the command must be exactly one simple command (a shell's `-c` script is
//! parsed with tree-sitter and must hold one plain command: no `;`, `&&`,
//! `|`, substitutions, expansions, redirections or assignments), its program,
//! after the wrappers `env`, `sudo`, `exec`, `command` and `nohup` (with only
//! the options listed here), must be named exactly (file name) as one of
//! [`KNOWN_TOOLS`], and the program and each wrapper must be a bare name or
//! a path in one of [`SYSTEM_DIRS`] (not a copy in a folder the agent can
//! write). Anything else, including anything this can't resolve, is not
//! offered.

use codex_sandboxing::user_namespaces::KNOWN_TOOLS;
use codex_shell_command::bash::extract_bash_command;
use codex_shell_command::bash::parse_shell_script_into_commands;

/// Folders a program may be named in by path: the system's, and Nix's
/// system-wide profiles. A path anywhere else (`./podman`, `/tmp/podman`,
/// `~/bin/podman`) is not resolved.
const SYSTEM_DIRS: [&str; 7] = [
    "/usr/bin",
    "/bin",
    "/usr/sbin",
    "/sbin",
    "/usr/local/bin",
    "/run/current-system/sw/bin",
    "/nix/var/nix/profiles/default/bin",
];

/// The known tool `command` runs, if it resolves to exactly one (see the
/// module docs).
pub(crate) fn known_tool(command: &[String]) -> Option<&'static str> {
    let words = match extract_bash_command(command) {
        Some((_, script)) => {
            let mut commands = parse_shell_script_into_commands(script)?;
            if commands.len() != 1 {
                return None;
            }
            commands.pop()?
        }
        None => command.to_vec(),
    };
    program_after_wrappers(&words)
}

/// The known tool `words` (one simple command) runs, after the wrappers.
fn program_after_wrappers(mut words: &[String]) -> Option<&'static str> {
    loop {
        let (first, rest) = words.split_first()?;
        let name = file_name(first)?;
        // The wrapper's options without and with a value (a user, group or
        // name, never a command).
        let (flags, takes_value): (&[&str], &[&str]) = match name {
            "env" => (&["-i", "--ignore-environment"], &["-u", "--unset"]),
            "sudo" => (&["-n", "--non-interactive", "-E", "-H"], &["-u", "-g"]),
            "exec" => (&["-c", "-l", "-cl", "-lc"], &["-a"]),
            "command" => (&["-p"], &[]),
            "nohup" => (&[], &[]),
            _ => return KNOWN_TOOLS.iter().copied().find(|tool| *tool == name),
        };
        let mut at = 0;
        while let Some(option) = rest.get(at).map(String::as_str) {
            if option == "--" {
                at += 1;
                break;
            }
            if !option.starts_with('-') {
                break;
            }
            at += 1;
            if takes_value.contains(&option) {
                rest.get(at)?;
                at += 1;
            } else if !(flags.contains(&option)
                || (name == "env" && option.starts_with("--unset=")))
            {
                // An option this doesn't know (`env -S`, `sudo -s`,
                // `command -v`): not resolved.
                return None;
            }
        }
        words = &rest[at..];
    }
}

/// The file name `word` runs, where it is a bare name or a path in one of
/// [`SYSTEM_DIRS`]; `None` for an assignment, an empty word or any other
/// path.
fn file_name(word: &str) -> Option<&str> {
    if word.is_empty() || word.contains('=') || word.starts_with('-') {
        return None;
    }
    match word.rsplit_once('/') {
        None => Some(word),
        Some((folder, name)) => (SYSTEM_DIRS.contains(&folder) && !name.is_empty()).then_some(name),
    }
}

#[cfg(test)]
#[path = "user_namespaces_known_tool_tests.rs"]
mod tests;
