//! #623 (milestone "Aggressive ships" re-run, AGG-R13): under Aggressive an
//! agent command may not create a link to the project's `.codex` or
//! `.agents` (or into Corbanu's own read-only files): they are always
//! read-only (#520), and a link to them is not created either (Travis,
//! 2026-10-13). Writes through such a link were already refused; now the
//! command is refused before it runs, so nothing is created, and the person
//! is told why.
//!
//! No operating-system sandbox can refuse a symbolic link by where it points
//! (Seatbelt filters only by the link's name or type; Landlock and seccomp
//! cannot read the target), so the command decides before anything runs:
//! its text for agent commands (see [`parse::link_targets`]), and, where the
//! zsh-fork shell intercepts each program a script starts, that program's
//! own arguments (so a script file's `ln` is seen there too).
//!
//! Out of reach: a target built at run time (`ln -s "$(cat f)" x`), a link
//! made by a script file without zsh-fork, keys typed into a running shell
//! (`write_stdin`) and remote environments. Writes through such a link stay
//! refused by the sandbox; a hard link to a protected file is refused by the
//! sandbox itself.

use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use codex_utils_absolute_path::AbsolutePathBuf;

use super::protected_reads::shown_path;
use super::protected_surface::ReadDenials;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;

#[path = "protected_links_parse.rs"]
mod parse;

use parse::Candidate;
#[cfg(test)]
use parse::calls_link;
use parse::link_targets;

/// What a person is told when such a command is refused.
pub(crate) fn explanation(target: &str) -> String {
    format!(
        "Security level Aggressive refused this command before it ran: it creates a link to \
         `{}`, and the project's .codex and .agents folders (agent settings) and Corbanu's own \
         files are always read-only under Aggressive, so no link to them is created either \
         (nothing was created). This cannot be granted, not even once. To allow it, a person can \
         switch to Permissive in /security (it applies after a restart).",
        shown_path(target)
    )
}

/// The refusal for `command` (run in `cwd`) under Aggressive, if it creates
/// a link to a path no grant lifts; also shown to the person and listed
/// under the inspector's recent denials.
pub(crate) async fn refusal(
    session: &Session,
    turn: &TurnContext,
    command: &[String],
    cwd: &Path,
) -> Option<String> {
    let candidates = link_targets(command);
    let first = candidates.first()?.target.clone();
    let inputs = Inputs::capture(session, turn)?;
    let cwd = cwd.to_path_buf();
    let found = match tokio::time::timeout(
        LOOKUP_TIMEOUT,
        tokio::task::spawn_blocking(move || inputs.protected_target(&candidates, &cwd)),
    )
    .await
    {
        Ok(Ok(found)) => found?,
        // The lookups did not finish: refused (nothing is created), naming
        // the first link target.
        Ok(Err(_)) | Err(_) => first,
    };
    let refusal = explanation(&found);
    super::protected_reads::notify(session, turn, refusal.clone()).await;
    super::inspection::record_refusal(
        session.thread_id(),
        "creating a link to the project's .codex or .agents, or Corbanu's files",
        "refused before it ran; never granted under Aggressive",
    );
    Some(refusal)
}

/// How long the path lookups may take before the command is refused anyway.
const LOOKUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// What the protected paths are built from (see `protected_reads`).
struct Inputs {
    codex_home: PathBuf,
    turn_cwd: AbsolutePathBuf,
    workspace_roots: Vec<AbsolutePathBuf>,
    profile: codex_protocol::models::PermissionProfile,
    user_home: Option<PathBuf>,
}

impl Inputs {
    /// Only under Aggressive, for a local command.
    fn capture(session: &Session, turn: &TurnContext) -> Option<Self> {
        session
            .services
            .model_client()
            .post_taint_state()
            .filter(|state| state.level == codex_security_policy::SecurityLevel::Aggressive)?;
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

    /// The first candidate, as spelled, that lands in a path no grant lifts.
    fn protected_target(&self, candidates: &[Candidate], cwd: &Path) -> Option<String> {
        let denials = ReadDenials::for_turn(
            &self.codex_home,
            &self.turn_cwd,
            &self.workspace_roots,
            &self.profile,
        );
        let mut kept: Vec<AbsolutePathBuf> = denials.never_lifted_paths().cloned().collect();
        let real: Vec<AbsolutePathBuf> =
            kept.iter().filter_map(ReadDenials::real_location).collect();
        kept.extend(real);
        let writable: Vec<&AbsolutePathBuf> = denials.home_writable_paths().collect();
        candidates.iter().find_map(|candidate| {
            bases(candidate, cwd)
                .iter()
                .flat_map(|base| self.resolve(&candidate.target, base))
                .any(|path| lands_in(&path, &kept, &writable))
                .then(|| candidate.target.clone())
        })
    }

    /// Where `word` points from `base`: as written (`..` taken from the text)
    /// and as the system reads it (`..` taken after following each link on
    /// the way, `ln -s evil/../.agents` with `evil` a link).
    fn resolve(&self, word: &str, base: &Path) -> Vec<AbsolutePathBuf> {
        let codex_home = self.codex_home.to_string_lossy();
        let base_text = base.to_string_lossy();
        let mut expanded = word.to_string();
        for name in ["CODEX_HOME", "CORBANU_HOME", "PFTERMINAL_HOME"] {
            for spelling in [format!("${{{name}}}"), format!("${name}")] {
                expanded = expanded.replace(&spelling, &codex_home);
            }
        }
        // The shell's folder.
        for spelling in ["${PWD}", "$PWD", "$(pwd)", "`pwd`"] {
            expanded = expanded.replace(spelling, &base_text);
        }
        if let Some(home) = self.user_home.as_deref().map(Path::to_string_lossy) {
            for spelling in ["${HOME}", "$HOME"] {
                expanded = expanded.replace(spelling, &home);
            }
            if expanded == "~" {
                expanded = home.into_owned();
            } else if let Some(rest) = expanded.strip_prefix("~/") {
                expanded = format!("{home}/{rest}");
            }
        }
        if expanded.is_empty() || expanded.contains('$') || expanded.starts_with('~') {
            return Vec::new();
        }
        let written = AbsolutePathBuf::resolve_path_against_base(&expanded, base);
        let mut resolved = vec![written];
        if let Some(physical) = physical(base, Path::new(&expanded))
            && !resolved.contains(&physical)
        {
            resolved.push(physical);
        }
        resolved
    }
}

/// The folders a candidate's relative target may be read from.
fn bases(candidate: &Candidate, cwd: &Path) -> Vec<PathBuf> {
    let mut bases = vec![cwd.to_path_buf()];
    for folder in &candidate.link_folders {
        let folder = cwd.join(folder);
        if !bases.contains(&folder) {
            bases.push(folder);
        }
    }
    bases
}

/// `path` from `base`, each existing name followed to where it links before
/// the next `..` applies, as the system reads it. Bounded by the path's
/// length; `None` where a name cannot be read.
fn physical(base: &Path, path: &Path) -> Option<AbsolutePathBuf> {
    let mut current = if path.is_absolute() {
        PathBuf::new()
    } else {
        std::fs::canonicalize(base).unwrap_or_else(|_| base.to_path_buf())
    };
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => current.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                current.pop();
            }
            Component::Normal(name) => {
                current.push(name);
                if let Ok(real) = std::fs::canonicalize(&current) {
                    current = real;
                }
            }
        }
    }
    AbsolutePathBuf::from_absolute_path(current).ok()
}

/// Whether `path`, as spelled, through a link on the way or where it
/// finally points, is in one of `kept` and not in a folder of `writable`
/// inside it (`worktrees` in the Corbanu home, #583).
fn lands_in(
    path: &AbsolutePathBuf,
    kept: &[AbsolutePathBuf],
    writable: &[&AbsolutePathBuf],
) -> bool {
    let mut spellings = vec![path.clone()];
    spellings.extend(ReadDenials::real_location(path));
    spellings.iter().any(|spelling| {
        kept.iter().any(|kept| {
            spelling.as_path().starts_with(kept.as_path())
                && !writable.iter().any(|folder| {
                    spelling.as_path().starts_with(folder.as_path())
                        && folder.as_path().starts_with(kept.as_path())
                })
        })
    })
}

#[cfg(test)]
#[path = "protected_links_tests.rs"]
mod tests;
