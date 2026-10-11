//! #442: under Aggressive, reads of protected files (credential files such as
//! `.env`, `~/.ssh` or cloud and CLI tokens, and the Corbanu home) are never
//! granted, not even once. When a command fails because the sandbox denied
//! such a read, this explains why at that point, naming the security level
//! and how a person can change it, instead of leaving a bare "Operation not
//! permitted".
//!
//! Explanation only: it never changes what runs or what is denied.

use std::path::Path;
use std::path::PathBuf;

use codex_protocol::permissions::ReadDenyMatcher;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::WarningEvent;
use codex_security_policy::SecurityLevel;

use crate::session::session::Session;
use crate::session::turn_context::TurnContext;

/// Output that says the operating system refused access.
const DENIAL_MARKERS: [&str; 2] = ["operation not permitted", "permission denied"];

/// The explanation shown when `path` could not be read.
pub(crate) fn explanation(path: &str) -> String {
    format!(
        "Security level Aggressive blocked reading `{path}`: agent commands can never read \
         protected files (credential files such as .env, SSH, cloud and CLI tokens, and the \
         Corbanu home), and this cannot be granted, not even once. To allow it, a person can \
         switch to Permissive in /security (it applies after a restart)."
    )
}

/// The explanation for a command that just failed, when it failed because
/// the session's Aggressive rules denied reading a protected path it names.
pub(crate) fn denial_explanation(
    session: &Session,
    turn: &TurnContext,
    command: &[String],
    cwd: &Path,
    exit_code: Option<i32>,
    output: &str,
) -> Option<String> {
    if exit_code == Some(0) || !mentions_denial(output) {
        return None;
    }
    session
        .services
        .model_client()
        .post_taint_state()
        .filter(|state| state.level == SecurityLevel::Aggressive)?;
    let policy = super::protected_surface::post_taint_read_policy(session, turn)?;
    let matcher = ReadDenyMatcher::new(&policy, cwd)?;
    let user_home = dirs::home_dir();
    denied_word(
        command,
        cwd,
        user_home.as_deref(),
        turn.config.codex_home.as_path(),
        |path| matcher.is_read_denied(path),
    )
    .map(|word| explanation(&word))
}

/// Shows `explanation` to the person as a warning beside the failed command.
pub(crate) async fn notify(session: &Session, turn: &TurnContext, explanation: String) {
    session
        .send_event(
            turn,
            EventMsg::Warning(WarningEvent {
                message: explanation,
            }),
        )
        .await;
}

fn mentions_denial(output: &str) -> bool {
    let output = output.to_ascii_lowercase();
    DENIAL_MARKERS.iter().any(|marker| output.contains(marker))
}

/// The first word of `command` (shell scripts split into words) that names a
/// path `denied` reports as unreadable, as the command spelled it.
pub(super) fn denied_word(
    command: &[String],
    cwd: &Path,
    user_home: Option<&Path>,
    codex_home: &Path,
    denied: impl Fn(&Path) -> bool,
) -> Option<String> {
    command
        .iter()
        .flat_map(|argument| {
            shlex::split(argument)
                .unwrap_or_else(|| argument.split_whitespace().map(str::to_string).collect())
        })
        .flat_map(|word| {
            // `--file=.env`, `<.env`, `.env;`: the path part of the word.
            let word = word.trim_matches(|c: char| matches!(c, ';' | '|' | '&' | '(' | ')'));
            let word = word.trim_start_matches(['<', '>']);
            let mut parts = vec![word.to_string()];
            if let Some((_, value)) = word.split_once('=') {
                parts.push(value.to_string());
            }
            parts
        })
        .filter(|word| !word.is_empty() && !word.starts_with('-'))
        .find(|word| {
            resolve(word, cwd, user_home, codex_home).is_some_and(|path| denied(path.as_path()))
        })
}

fn resolve(word: &str, cwd: &Path, user_home: Option<&Path>, codex_home: &Path) -> Option<PathBuf> {
    let codex_home = codex_home.to_string_lossy();
    let mut expanded = word.to_string();
    for name in ["CODEX_HOME", "CORBANU_HOME", "PFTERMINAL_HOME"] {
        for spelling in [format!("${{{name}}}"), format!("${name}")] {
            expanded = expanded.replace(&spelling, &codex_home);
        }
    }
    if let Some(home) = user_home.map(Path::to_string_lossy) {
        for spelling in ["${HOME}", "$HOME"] {
            expanded = expanded.replace(spelling, &home);
        }
        if expanded == "~" {
            expanded = home.into_owned();
        } else if let Some(rest) = expanded.strip_prefix("~/") {
            expanded = format!("{home}/{rest}");
        }
    }
    if expanded.contains('$') || expanded.starts_with('~') {
        return None;
    }
    Some(cwd.join(expanded))
}

#[cfg(test)]
#[path = "protected_reads_tests.rs"]
mod tests;
