//! #442: under Aggressive, reads of protected files (credential files such as
//! `.env`, `~/.ssh` or cloud and CLI tokens, and the Corbanu home) are never
//! granted, not even once. This module finds the protected path a command
//! names, so that:
//! - a command that failed because the sandbox denied such a read is
//!   explained where it failed (the level, never grantable, how to change
//!   it), instead of a bare "Operation not permitted";
//! - the approval prompt for such a command offers no grant and says why;
//! - #620: the same for a start-up file outside the current folder
//!   (`~/.zshrc`): Aggressive's sandbox never writes there, so no grant
//!   could let the command write it;
//! - #622: such a denial is listed under the inspector's recent denials.
//!
//! Explanation only: it never changes what runs or what is denied (it only
//! withholds a grant offer that could not apply). The path
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
use super::tainted_action::USER_PERSISTENCE;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;

/// Output that says the operating system refused access (on Linux a write
/// outside the writable folders reports a read-only file system; #620).
const DENIAL_MARKERS: [&str; 3] = [
    "operation not permitted",
    "permission denied",
    "read-only file system",
];

/// How long the path lookups may take before no explanation is given.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);

/// The explanation shown when `path` could not be accessed.
pub(crate) fn explanation(path: &str) -> String {
    let path = shown_path(path);
    format!(
        "Security level Aggressive blocked access to `{path}`: agent commands can never read \
         protected files (credential files such as .env, .env.* and .envrc, also ones created \
         during the session; SSH, cloud and CLI tokens; and Corbanu's sign-in, vault, config \
         and history), and this cannot be granted, not even \
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

/// #620: the explanation for a start-up file outside the current folder
/// (`~/.zshrc`): Aggressive's sandbox writes only inside the current folder,
/// and a grant lifts the start-up file protections only there.
pub(crate) fn startup_explanation(path: &str) -> String {
    format!(
        "Security level Aggressive blocks writing `{}`: it is a start-up file outside the \
         current folder, and agent commands can write only inside the current folder, even \
         when you approve them; this cannot be granted, not even once (Grant once lifts the \
         start-up file protections only inside the current folder). To allow it, a person can \
         switch to Permissive in /security (it applies after a restart).",
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

    /// #620: whether `path` is one of the user's start-up files (`~/.zshrc`,
    /// login items; [`USER_PERSISTENCE`]) that the session's sandbox cannot
    /// write at all: outside the current folder, so no grant could let a
    /// command write it. One reached through a link counts where the link
    /// points.
    fn unwritable_startup_file(&self, path: &Path) -> bool {
        let (Some(user_home), Ok(path)) = (
            self.user_home.as_deref(),
            AbsolutePathBuf::from_absolute_path(path),
        ) else {
            return false;
        };
        if !USER_PERSISTENCE
            .iter()
            .any(|entry| path.as_path().starts_with(user_home.join(entry)))
        {
            return false;
        }
        let target = ReadDenials::real_location(&path).unwrap_or(path);
        !self
            .profile
            .file_system_sandbox_policy()
            .can_write_path_with_cwd(target.as_path(), self.turn_cwd.as_path())
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
/// command names (or the command's folder is itself protected), or of a
/// protected path the line itself reports (#621: a path the command built
/// at run time, such as an interpreter's `os.listdir(...)`).
///
/// #622: a denial explained here is also listed under the inspector's
/// recent denials.
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
    let found = off_thread(move || {
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
                Some(&denial_lines),
            )
        };
        let in_lines = |lines: &[String], accept: &dyn Fn(&Path) -> bool| {
            path_in_lines(
                &command,
                &cwd,
                inputs.user_home.as_deref(),
                &inputs.codex_home,
                lines,
                accept,
            )
        };
        word(&|path| matcher.is_read_denied(path))
            .map(ProtectedNote::Read)
            .or_else(|| word(&|path| matcher.is_listing_denied(path)).map(ProtectedNote::Listing))
            // #506: `ls` run in the Corbanu home itself (the denial names the
            // folder as `.` or by its name).
            .or_else(|| {
                let name = cwd.file_name()?.to_str()?.to_string();
                (matcher.is_listing_denied(&cwd)
                    && denial_lines.iter().any(|line| {
                        line.contains(&name) || line.contains(" .:") || line.contains("'.'")
                    }))
                .then(|| ProtectedNote::Listing(cwd.display().to_string()))
            })
            // #554: a folder above the home listed recursively (`find ~`,
            // `ls -aR ~`): the denial line names the home below it.
            .or_else(|| {
                unlistable_path_in_lines(
                    &command,
                    &cwd,
                    inputs.user_home.as_deref(),
                    &inputs.codex_home,
                    &denial_lines,
                    &|path| matcher.is_listing_denied(path) && !matcher.is_read_denied(path),
                )
                .map(ProtectedNote::Listing)
            })
            // #620: a start-up file outside the current folder.
            .or_else(|| {
                word(&|path| inputs.unwritable_startup_file(path)).map(ProtectedNote::Startup)
            })
            // #621: a protected path the denial line reports that no word
            // of the command names (built at run time). Bounded: only the
            // first lines are looked at.
            .or_else(|| {
                let lines = &denial_lines[..denial_lines.len().min(MAX_REPORTED_LINES)];
                in_lines(lines, &|path| matcher.is_read_denied(path)).map(ProtectedNote::Read)
            })
    })
    .await?;
    super::inspection::record_sandbox_denial(session.thread_id(), found.inspected());
    Some(found.explanation())
}

/// Denial lines looked at for a path the command does not name (#621).
const MAX_REPORTED_LINES: usize = 32;

/// #554: the path a denial line reports, as it spells it, that `unlistable`
/// accepts. A relative path is taken from the command's folder or from a
/// folder the command names (`cd ~ && find .`).
pub(super) fn unlistable_path_in_lines(
    command: &[String],
    cwd: &Path,
    user_home: Option<&Path>,
    codex_home: &Path,
    denial_lines: &[String],
    unlistable: &dyn Fn(&Path) -> bool,
) -> Option<String> {
    // Only the Corbanu home is unlistable: skip the other lines (`find ~`
    // can report many) without looking anything up.
    let name = codex_home.file_name()?.to_str()?;
    let lines: Vec<String> = denial_lines
        .iter()
        .filter(|line| line.contains(name))
        .cloned()
        .collect();
    path_in_lines(command, cwd, user_home, codex_home, &lines, unlistable)
}

/// The path one of `denial_lines` reports, as it spells it, that `accept`
/// accepts; relative paths as in [`unlistable_path_in_lines`].
fn path_in_lines(
    command: &[String],
    cwd: &Path,
    user_home: Option<&Path>,
    codex_home: &Path,
    denial_lines: &[String],
    accept: &dyn Fn(&Path) -> bool,
) -> Option<String> {
    if denial_lines.is_empty() {
        return None;
    }
    let mut bases = vec![cwd.to_path_buf()];
    bases.extend(
        command
            .iter()
            .flat_map(|argument| {
                shlex::split(argument)
                    .unwrap_or_else(|| argument.split_whitespace().map(str::to_string).collect())
            })
            .map(|word| {
                word.trim_matches(|c: char| matches!(c, ';' | '|' | '&' | '(' | ')'))
                    .to_string()
            })
            .filter(|word| !word.is_empty() && !word.starts_with('-'))
            .filter_map(|word| resolve(&word, cwd, user_home, codex_home)),
    );
    denial_lines
        .iter()
        .flat_map(|line| paths_in_denial_line(line))
        .find(|spelled| {
            bases
                .iter()
                .any(|base| accept(&base.join(spelled.as_str())))
        })
}

/// The paths a denial line may name: quoted (`find: ‘/x’: Permission
/// denied`, `ls: cannot open directory '/x': …`) or between the program and
/// the reason (`ls: /x: Operation not permitted`).
fn paths_in_denial_line(line: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for (open, close) in [('‘', '’'), ('\'', '\''), ('"', '"'), ('`', '\'')] {
        let mut rest = line;
        while let Some(start) = rest.find(open) {
            let after = &rest[start + open.len_utf8()..];
            let Some(end) = after.find(close) else {
                break;
            };
            paths.push(after[..end].to_string());
            rest = &after[end + close.len_utf8()..];
        }
    }
    let fields: Vec<&str> = line.split(": ").collect();
    if fields.len() >= 3 {
        paths.push(fields[1..fields.len() - 1].join(": "));
    }
    paths.retain(|path| !path.trim().is_empty());
    paths
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
        ProtectedNote::Listing(_) | ProtectedNote::Startup(_) => None,
    }
}

/// What a command names that no grant opens under Aggressive.
pub(crate) enum ProtectedNote {
    /// A protected path it reads (#442).
    Read(String),
    /// A folder it lists that may not be listed (#506).
    Listing(String),
    /// A start-up file outside the current folder, which Aggressive's
    /// sandbox never writes (#620).
    Startup(String),
}

impl ProtectedNote {
    /// The explanation for the approval prompt, or where the command failed.
    pub(crate) fn explanation(&self) -> String {
        match self {
            Self::Read(path) => explanation(path),
            Self::Listing(path) => listing_explanation(path),
            Self::Startup(path) => startup_explanation(path),
        }
    }

    /// #622: what the inspector's recent denials say was refused (fixed
    /// text: the inspector never shows paths).
    fn inspected(&self) -> &'static str {
        match self {
            Self::Read(_) => "reading a protected file",
            Self::Listing(_) => "listing the Corbanu home",
            Self::Startup(_) => "writing a start-up file outside the current folder",
        }
    }
}

/// The protected path `command` names (#442), or else the folder it lists
/// that may not be listed (#506), or else a start-up file outside the
/// current folder it names (#620): no grant could apply to any of them.
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
            // #554: Windows does not enforce the listing denial (its sandbox
            // ignores the entry), so it says nothing about listings there.
            .or_else(|| {
                (!cfg!(windows))
                    .then(|| word(&|path| matcher.is_listing_denied(path)))
                    .flatten()
                    .map(ProtectedNote::Listing)
            })
            .or_else(|| {
                word(&|path| inputs.unwritable_startup_file(path)).map(ProtectedNote::Startup)
            })
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
    // An external sandbox takes no protected-path rules (and ignores
    // additional permissions), as in the orchestrator's check.
    if matches!(
        inputs.profile,
        codex_protocol::models::PermissionProfile::External { .. }
    ) {
        return None;
    }
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
        // Issue #513: nor a path no grant lifts for writing (#565: the
        // runtime entries of the Corbanu home and the project's `.codex` and
        // `.agents`). Other write protections wait for the run's grant to
        // be known.
        let protected = denials
            .apply(&inputs.profile)
            .map(|protected| protected.file_system_sandbox_policy());
        let check = super::protected_additional::Check {
            denials: &denials,
            protected: protected.as_ref(),
            reads: true,
            writes: super::protected_additional::ReadOnlyScope::NeverLifted,
        };
        super::protected_additional::opened(check, &additional, &cwds)
            .map(|opened| opened.refusal(SecurityLevel::Aggressive))
    })
    .await
}

/// #622: lists a command refused for additional permissions that would
/// open a protected path ([`additional_permission_refusal`]) under the
/// inspector's recent denials.
pub(crate) fn record_permission_refusal(session: &Session) {
    super::inspection::record_refusal(
        session.thread_id(),
        "additional permissions naming a protected path",
        "refused before it ran; never granted under Aggressive",
    );
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
        .flat_map(|whole| {
            // `--file=.env`, `<.env`, `.env;`: the path part of the word.
            let word = whole.trim_matches(|c: char| matches!(c, ';' | '|' | '&' | '(' | ')'));
            let word = word.trim_start_matches(['<', '>']);
            let mut parts = vec![word.to_string()];
            if let Some((_, value)) = word.split_once('=') {
                parts.push(value.to_string());
            }
            // #621: paths inside code an interpreter runs (`python3 -c`,
            // `node -e`, `perl -e`, `ruby -e`).
            parts.extend(embedded_paths(&whole));
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

/// Characters that end a path inside code.
const CODE_DELIMITERS: [char; 16] = [
    '\'', '"', '`', '(', ')', '[', ']', '{', '}', ',', ';', '+', ' ', '\t', '\n', '\r',
];

/// Paths looked at inside one word that is code.
const MAX_EMBEDDED_PATHS: usize = 64;

/// #621: the path literals in `word` when it is code rather than a path
/// (`import os; print(os.listdir('/x'))`): the quoted strings, and the
/// tokens that start like a path (`/`, `~`, `$`). Bounded; a path built at
/// run time is not found here, and is explained where the sandbox denies it
/// instead.
fn embedded_paths(word: &str) -> Vec<String> {
    if !word.contains(CODE_DELIMITERS) {
        return Vec::new();
    }
    let mut found: Vec<String> = Vec::new();
    let quoted = ['\'', '"', '`'].into_iter().flat_map(|quote| {
        word.split(quote)
            .skip(1)
            .step_by(2)
            .map(str::to_string)
            .collect::<Vec<_>>()
    });
    let tokens = word
        .split(CODE_DELIMITERS)
        .filter(|token| token.starts_with(['/', '~', '$']))
        .map(str::to_string);
    for path in quoted.chain(tokens) {
        if found.len() == MAX_EMBEDDED_PATHS {
            break;
        }
        let path = path.trim().to_string();
        if !path.is_empty() && path != word && !found.contains(&path) {
            found.push(path);
        }
    }
    found
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
