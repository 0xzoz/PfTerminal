//! #442: under Aggressive, reads of protected files (credential files such as
//! `.env`, `~/.ssh` or cloud and CLI tokens, and the Corbanu home) are never
//! granted, not even once. This module finds the protected path a command
//! names, so that:
//! - a command that failed because the sandbox denied such a read is
//!   explained where it failed (the level, never grantable, how to change
//!   it), instead of a bare "Operation not permitted";
//! - the approval prompt for such a command offers no grant and says why.
//!
//! Explanation only: it never changes what runs or what is denied. The path
//! lookups run off the async thread, bounded by a timeout.

use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use codex_protocol::models::PermissionProfile;
use codex_protocol::permissions::ReadDenyMatcher;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::WarningEvent;
use codex_security_policy::SecurityLevel;
use codex_utils_absolute_path::AbsolutePathBuf;

use super::protected_surface::ReadDenials;
use super::tainted_action::PolicyBinding;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;

/// Output that says the operating system refused access.
const DENIAL_MARKERS: [&str; 2] = ["operation not permitted", "permission denied"];

/// How long the path lookups may take before no explanation is given.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);

/// The explanation shown when `path` could not be accessed.
pub(crate) fn explanation(path: &str) -> String {
    let path = shown_path(path);
    format!(
        "Security level Aggressive blocked access to `{path}`: agent commands can never read \
         protected files (credential files such as .env, SSH, cloud and CLI tokens, and \
         Corbanu's sign-in, vault, config and history), and this cannot be granted, not even \
         once. To allow it, a person can switch to Permissive in /security (it applies after a \
         restart)."
    )
}

/// #506: the explanation shown when listing the folder `path` was denied.
pub(crate) fn listing_explanation(path: &str) -> String {
    format!(
        "Security level Aggressive blocked listing `{}`: agent commands can never list the \
         Corbanu home or read its protected files (Corbanu's sign-in, vault, config and \
         history), and this cannot be granted, not even once. To allow it, a person can switch \
         to Permissive in /security (it applies after a restart).",
        shown_path(path)
    )
}

/// `path` as an explanation shows it: it is the command's own spelling, so
/// escaped and bounded.
pub(crate) fn shown_path(path: &str) -> String {
    let mut path = super::grant_offer::escape_controls(path);
    if path.chars().count() > MAX_SHOWN_PATH_CHARS {
        path = path.chars().take(MAX_SHOWN_PATH_CHARS).collect::<String>() + "…";
    }
    path
}

/// Longest path shown in an explanation.
const MAX_SHOWN_PATH_CHARS: usize = 200;

/// What the session's read policy is built from, captured on the async
/// thread; building it reads folders, so that happens off it.
struct ReadPolicyInputs {
    codex_home: PathBuf,
    turn_cwd: AbsolutePathBuf,
    workspace_roots: Vec<AbsolutePathBuf>,
    profile: PermissionProfile,
    user_home: Option<PathBuf>,
}

impl ReadPolicyInputs {
    /// Only under a live (or configured) Aggressive level, not when the
    /// policy is unreadable and Aggressive is only the fallback.
    fn capture(session: &Session, turn: &TurnContext) -> Option<Self> {
        session
            .services
            .model_client()
            .post_taint_state()
            .filter(|state| {
                state.level == SecurityLevel::Aggressive
                    && !matches!(state.policy, PolicyBinding::Unavailable)
            })?;
        // Local paths only: a remote command's paths are not this host's.
        if turn
            .environments
            .primary()
            .is_some_and(|environment| environment.environment.is_remote())
        {
            return None;
        }
        #[allow(deprecated)]
        let turn_cwd = turn.cwd.clone();
        let workspace_roots = turn
            .environments
            .primary()
            .map(|environment| {
                environment
                    .workspace_roots()
                    .iter()
                    .filter_map(|root| root.to_abs_path().ok())
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            codex_home: turn.config.codex_home.to_path_buf(),
            turn_cwd,
            workspace_roots,
            profile: turn.config.permissions.permission_profile().clone(),
            user_home: dirs::home_dir(),
        })
    }

    /// The deny-read matcher of the session's read policy; `None` for a
    /// profile that cannot take the rules (an external sandbox).
    fn matcher(&self, cwd: &Path) -> Option<ReadDenyMatcher> {
        let mut denials = ReadDenials::for_turn(
            &self.codex_home,
            &self.turn_cwd,
            &self.workspace_roots,
            &self.profile,
        );
        // Captured only under Aggressive (see `capture`).
        denials.deny_home_listing();
        let policy = denials.apply(&self.profile)?.file_system_sandbox_policy();
        // A malformed pattern denies every path; explain nothing then.
        ReadDenyMatcher::try_new(&policy, cwd).ok().flatten()
    }
}

async fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> Option<T> + Send + 'static,
) -> Option<T> {
    tokio::time::timeout(LOOKUP_TIMEOUT, tokio::task::spawn_blocking(work))
        .await
        .ok()?
        .ok()?
}

/// The explanation for a command that just failed with a known non-zero
/// exit, when an output line reports a denial of a protected path the
/// command names (or the command's folder is itself protected).
pub(crate) async fn denial_explanation(
    session: &Session,
    turn: &TurnContext,
    command: &[String],
    cwd: &Path,
    exit_code: Option<i32>,
    output: &str,
) -> Option<String> {
    if !matches!(exit_code, Some(code) if code != 0) {
        return None;
    }
    let denial_lines: Vec<String> = output
        .lines()
        .filter(|line| {
            let line = line.to_ascii_lowercase();
            DENIAL_MARKERS.iter().any(|marker| line.contains(marker))
        })
        .map(str::to_string)
        .collect();
    if denial_lines.is_empty() {
        return None;
    }
    let inputs = ReadPolicyInputs::capture(session, turn)?;
    let (command, cwd) = (command.to_vec(), cwd.to_path_buf());
    off_thread(move || {
        let matcher = inputs.matcher(&cwd)?;
        if matcher.is_read_denied(&cwd) {
            return Some(explanation(&cwd.display().to_string()));
        }
        let word = |denied: &dyn Fn(&Path) -> bool| {
            denied_word(
                &command,
                &cwd,
                inputs.user_home.as_deref(),
                &inputs.codex_home,
                denied,
                Some(&denial_lines),
            )
        };
        word(&|path| matcher.is_read_denied(path))
            .map(|word| explanation(&word))
            .or_else(|| {
                word(&|path| matcher.is_listing_denied(path)).map(|word| listing_explanation(&word))
            })
            // #506: `ls` run in the Corbanu home itself (the denial names the
            // folder as `.` or by its name).
            .or_else(|| {
                let name = cwd.file_name()?.to_str()?.to_string();
                (matcher.is_listing_denied(&cwd)
                    && denial_lines.iter().any(|line| {
                        line.contains(&name) || line.contains(" .:") || line.contains("'.'")
                    }))
                .then(|| listing_explanation(&cwd.display().to_string()))
            })
    })
    .await
}

/// #442: the protected path `command` names, under Aggressive: a grant for
/// it would never apply, so none is offered.
#[cfg(test)]
pub(crate) async fn protected_path_in_command(
    session: &Session,
    turn: &TurnContext,
    command: &[String],
    cwd: &Path,
) -> Option<String> {
    match protected_note_in_command(session, turn, command, cwd).await? {
        ProtectedNote::Read(path) => Some(path),
        ProtectedNote::Listing(_) => None,
    }
}

/// What a command names that no grant opens under Aggressive.
pub(crate) enum ProtectedNote {
    /// A protected path it reads (#442).
    Read(String),
    /// A folder it lists that may not be listed (#506).
    Listing(String),
}

impl ProtectedNote {
    /// The explanation for the approval prompt.
    pub(crate) fn explanation(&self) -> String {
        match self {
            Self::Read(path) => explanation(path),
            Self::Listing(path) => listing_explanation(path),
        }
    }
}

/// The protected path `command` names (#442), or else the folder it lists
/// that may not be listed (#506).
pub(crate) async fn protected_note_in_command(
    session: &Session,
    turn: &TurnContext,
    command: &[String],
    cwd: &Path,
) -> Option<ProtectedNote> {
    let inputs = ReadPolicyInputs::capture(session, turn)?;
    let (command, cwd) = (command.to_vec(), cwd.to_path_buf());
    off_thread(move || {
        let matcher = inputs.matcher(&cwd)?;
        if matcher.is_read_denied(&cwd) {
            return Some(ProtectedNote::Read(cwd.display().to_string()));
        }
        let word = |denied: &dyn Fn(&Path) -> bool| {
            denied_word(
                &command,
                &cwd,
                inputs.user_home.as_deref(),
                &inputs.codex_home,
                denied,
                /*denial_lines*/ None,
            )
        };
        word(&|path| matcher.is_read_denied(path))
            .map(ProtectedNote::Read)
            .or_else(|| word(&|path| matcher.is_listing_denied(path)).map(ProtectedNote::Listing))
    })
    .await
}

/// Issue #245: under Aggressive, the explanation for a request whose own
/// additional permissions would open a protected path. Checked before the
/// request is validated, so it is refused with this explanation even where
/// permission requests are off (as Aggressive keeps them).
pub(crate) async fn additional_permission_refusal(
    session: &Session,
    turn: &TurnContext,
    additional: Option<&codex_protocol::models::AdditionalPermissionProfile>,
    cwd: &Path,
) -> Option<String> {
    let additional = additional?.clone();
    let inputs = ReadPolicyInputs::capture(session, turn)?;
    let cwd = cwd.to_path_buf();
    off_thread(move || {
        let denials = ReadDenials::for_turn(
            &inputs.codex_home,
            &inputs.turn_cwd,
            &inputs.workspace_roots,
            &inputs.profile,
        );
        let mut cwds = vec![cwd.as_path(), inputs.turn_cwd.as_path()];
        cwds.extend(inputs.workspace_roots.iter().map(AbsolutePathBuf::as_path));
        super::protected_additional::opened_protected_path(&denials, &additional, &cwds)
            .map(|path| explanation(&path))
    })
    .await
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

/// The first word of `command` (shell scripts split into words) that names a
/// path `denied` reports as unreadable, as the command spelled it. With
/// `denial_lines`, only a word whose file name one of those lines mentions.
pub(super) fn denied_word(
    command: &[String],
    cwd: &Path,
    user_home: Option<&Path>,
    codex_home: &Path,
    denied: impl Fn(&Path) -> bool,
    denial_lines: Option<&[String]>,
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
            let Some(path) = resolve(word, cwd, user_home, codex_home) else {
                return false;
            };
            // As spelled, or as resolved (`$CODEX_HOME`, `dir/..`; #506).
            let resolved = AbsolutePathBuf::from_absolute_path(&path)
                .ok()
                .and_then(|path| {
                    path.as_path()
                        .file_name()
                        .map(std::ffi::OsStr::to_os_string)
                });
            let names = [
                Path::new(word)
                    .file_name()
                    .map(std::ffi::OsStr::to_os_string),
                resolved,
            ];
            denial_lines.is_none_or(|lines| {
                names
                    .iter()
                    .flatten()
                    .filter_map(|name| name.to_str())
                    .any(|name| lines.iter().any(|line| line.contains(name)))
            }) && denied(path.as_path())
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
