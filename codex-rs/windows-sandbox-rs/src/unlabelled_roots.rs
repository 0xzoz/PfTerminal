//! #734: writable roots the legacy (unelevated) sandbox couldn't label Low
//! (#158). Such a root stays as it is, so sandboxed commands can't write
//! there; the reason was only in the sandbox log, so the user saw a plain
//! "access denied". Core reads this list after a sandboxed run and warns once
//! per thread. Display only: it never changes how commands run.

use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::MutexGuard;

/// Roots whose latest labelling in this process failed.
static UNLABELLED: LazyLock<Mutex<BTreeSet<PathBuf>>> = LazyLock::new(Default::default);

fn unlabelled() -> MutexGuard<'static, BTreeSet<PathBuf>> {
    UNLABELLED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Records whether `root` (canonical) could be labelled Low just now: a root
/// fixed since (its owner changed, say) stops being reported.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn record(root: &Path, labelled: bool) {
    let mut unlabelled = unlabelled();
    if labelled {
        unlabelled.remove(root);
    } else {
        unlabelled.insert(root.to_path_buf());
    }
}

/// The canonical writable roots that the legacy sandbox couldn't label Low
/// the last time it tried in this process (always empty off Windows).
pub fn unlabelled_writable_roots() -> Vec<PathBuf> {
    unlabelled().iter().cloned().collect()
}

/// What to tell the person about such a `root`.
pub fn unlabelled_writable_root_warning(root: &Path) -> String {
    format!(
        "`{}` is read-only to commands in the non-admin Windows sandbox, although it is a \
         writable folder: that sandbox runs commands at Low integrity, and the folder could not \
         be labelled Low. Labelling needs permission to change the folder's owner settings \
         (which owning it gives) on a file system that holds integrity labels, such as NTFS. To \
         let commands write there, own the folder, or switch to the default sandbox by running \
         /setup-default-sandbox, or set `sandbox = \"elevated\"` under `[windows]` in config.toml.",
        root.display()
    )
}

#[cfg(test)]
#[path = "unlabelled_roots_tests.rs"]
mod tests;
