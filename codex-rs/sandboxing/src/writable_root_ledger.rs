//! Issue #536 (Travis, 2026-10-12): a writable root must not follow a link
//! made after the session started, nor a link that leads outside the
//! writable roots holding it, at every level (Permissive included).
//!
//! The escape: a writable root that is configured but does not exist yet,
//! inside another writable root (`ws/target`), is created by a command as a
//! link (`ln -s /elsewhere ws/target`); from the next command on, write
//! access followed the link.
//!
//! Each writable root's real location (every link followed, see
//! [`real_location`]) is recorded the first time this process sees it,
//! normally when the session starts ([`record_writable_roots`]). Before each
//! sandboxed command ([`check_writable_roots`]) a root whose real location
//! changed since, or one held by another writable root that leads outside
//! every root holding it, refuses the command with a message saying why. The
//! sandboxes then grant writes at the recorded real location
//! ([`pinned_real_location`]), not wherever the root leads by the time they
//! are set up, so a link swapped in after the check grants nothing.
//!
//! A root's record lives as long as the process: a link made during one
//! session is still refused in the next session of the same process.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::MutexGuard;

use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::PinnedWritableRoot;
#[cfg(unix)]
use codex_protocol::permissions::real_location;
use codex_protocol::protocol::WritableRoot;

static LEDGER: LazyLock<Mutex<HashMap<PathBuf, PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn ledger() -> MutexGuard<'static, HashMap<PathBuf, PathBuf>> {
    LEDGER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Why a command was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WritableRootRefusal {
    /// The root leads somewhere else than when the session first saw it.
    Changed {
        root: PathBuf,
        was: PathBuf,
        now: PathBuf,
    },
    /// The root, held by another writable root, leads outside every root
    /// holding it.
    OutsideHolder {
        root: PathBuf,
        real: PathBuf,
        holder: PathBuf,
    },
}

impl fmt::Display for WritableRootRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Changed { root, was, now } => write!(
                f,
                "Refused to run the command: the writable folder `{}` now leads to `{}`, but it \
                 led to `{}` when the session started. A link made during the session cannot \
                 move write access. Remove the link (or restart Corbanu if you made it on \
                 purpose).",
                root.display(),
                now.display(),
                was.display(),
            ),
            Self::OutsideHolder { root, real, holder } => write!(
                f,
                "Refused to run the command: the writable folder `{}` is a link to `{}`, outside \
                 `{}`, the writable folder that holds it. A link cannot extend write access. \
                 Name the real folder instead, or remove the link.",
                root.display(),
                real.display(),
                holder.display(),
            ),
        }
    }
}

impl std::error::Error for WritableRootRefusal {}

/// Records the real location of each writable root of `policy` this process
/// has not seen yet. Called when a session starts.
pub fn record_writable_roots(policy: &FileSystemSandboxPolicy, cwd: &Path) {
    let roots = policy.get_writable_roots_with_cwd(cwd);
    let mut ledger = ledger();
    for root in roots {
        let real = real_path(root.root.as_path());
        ledger.entry(root.root.into_path_buf()).or_insert(real);
    }
}

/// Checks every writable root of `policy` before a sandboxed command (a root
/// seen for the first time is recorded now) and returns them with the real
/// locations the sandbox must grant.
pub fn check_writable_roots(
    policy: &FileSystemSandboxPolicy,
    cwd: &Path,
) -> Result<Vec<PinnedWritableRoot>, WritableRootRefusal> {
    let roots = policy.get_writable_roots_with_cwd(cwd);
    let reals: Vec<PathBuf> = roots
        .iter()
        .map(|root| real_path(root.root.as_path()))
        .collect();
    let mut ledger = ledger();
    let mut pinned = Vec::with_capacity(roots.len());
    for (index, root) in roots.iter().enumerate() {
        let now = &reals[index];
        let was = ledger
            .entry(root.root.to_path_buf())
            .or_insert_with(|| now.clone());
        if *was != *now {
            return Err(WritableRootRefusal::Changed {
                root: root.root.to_path_buf(),
                was: was.clone(),
                now: now.clone(),
            });
        }
        let holders: Vec<usize> = roots
            .iter()
            .enumerate()
            .filter(|(other, holder)| *other != index && holds(holder, &reals[*other], root))
            .map(|(other, _)| other)
            .collect();
        if let Some(first) = holders.first()
            && !holders
                .iter()
                .any(|holder| now.starts_with(&reals[*holder]))
        {
            return Err(WritableRootRefusal::OutsideHolder {
                root: root.root.to_path_buf(),
                real: now.clone(),
                holder: roots[*first].root.to_path_buf(),
            });
        }
        pinned.push(PinnedWritableRoot {
            root: root.root.clone(),
            real: now.clone(),
        });
    }
    Ok(pinned)
}

/// The real location recorded for `root`, if this process has seen it.
pub fn pinned_real_location(root: &Path) -> Option<PathBuf> {
    ledger().get(root).cloned()
}

/// Whether `holder` (whose real location is `holder_real`) holds `root`:
/// `root` is spelled inside it, or its folder really is inside it.
fn holds(holder: &WritableRoot, holder_real: &Path, root: &WritableRoot) -> bool {
    root.root != holder.root
        && (root.root.as_path().starts_with(holder.root.as_path())
            || root
                .root
                .as_path()
                .parent()
                .is_some_and(|parent| real_path(parent).starts_with(holder_real)))
}

#[cfg(unix)]
fn real_path(path: &Path) -> PathBuf {
    real_location(path)
}

#[cfg(not(unix))]
fn real_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

#[cfg(test)]
#[path = "writable_root_ledger_tests.rs"]
mod tests;
