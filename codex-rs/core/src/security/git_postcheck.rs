//! #874, #837 (Travis, 2026-10-16, D6): the post-command `.git` check.
//!
//! Where an agent command under Moderate or Aggressive can write a project's
//! `.git` (full access in your own settings; under the default workspace
//! sandbox `.git` is read-only), it can leave things there that your own git
//! later follows outside the sandbox: a link that git writes through
//! (`COMMIT_EDITMSG`, `MERGE_MSG`, `FETCH_HEAD`, `logs/*`), a hard link, rebase
//! or sequencer steps whose `exec` lines `git rebase --continue` runs, a
//! legacy remote definition (`remotes/`, `branches/`), or a replaced `.git`.
//!
//! Before such a command (once per session and repository) and after it, and
//! when the turn ends, this compares the relevant `.git` entries with the
//! last look. Anything new of those kinds is reported to the person with how
//! to remove it; nothing is removed. Until it is removed, a person's approval
//! does not lift the protected-file rules (`held`), so an approved
//! `git commit` cannot write through a planted link into a protected file.
//!
//! Cheap: `objects`, `refs` and `lfs` are not visited (git replaces their
//! files by renaming, it does not write through them), and a folder whose
//! modification time is unchanged is not listed again.

#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub(crate) use unix::*;

/// Elsewhere (Windows) there is no check: commands under Moderate and
/// Aggressive run in the Windows sandbox, which keeps `.git` read-only.
#[cfg(not(unix))]
mod other {
    use codex_protocol::ThreadId;
    use std::path::Path;
    use std::path::PathBuf;

    /// None are found here.
    #[derive(Clone, Debug)]
    pub(crate) enum Repo {}

    pub(crate) fn repos(_folders: &[&Path]) -> Vec<Repo> {
        Vec::new()
    }

    pub(crate) fn begin(_thread: ThreadId, _repos: &[Repo]) {}

    pub(crate) fn finish(_thread: ThreadId, _repos: &[Repo], _level: &str) -> Option<String> {
        None
    }

    pub(crate) fn recheck(_thread: ThreadId, _level: &str) -> Option<String> {
        None
    }

    pub(crate) fn held(_thread: ThreadId) -> Vec<PathBuf> {
        Vec::new()
    }

    impl Repo {
        pub(crate) fn probes(&self) -> Vec<PathBuf> {
            match *self {}
        }
    }
}

#[cfg(not(unix))]
pub(crate) use other::*;

#[cfg(all(test, unix))]
#[path = "git_postcheck_tests.rs"]
mod tests;
