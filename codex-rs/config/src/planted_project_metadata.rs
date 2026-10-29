//! #1506 item 2 (D262): `.codex` and `.agents` folders that were absent when
//! a non-admin (legacy) Windows sandbox session started and existed when it
//! ended. A sandboxed command may have planted them, so Corbanu Terminal
//! doesn't load them (project config, hooks, skills) until the user
//! confirms them. The records live with the sandbox's integrity-label
//! records, where sandboxed commands can't change them.

use std::path::Path;
use std::path::PathBuf;

/// Start of the warning, and of the disabled reason, for a project folder
/// that isn't loaded because it may have been planted.
pub const PLANTED_PROJECT_METADATA_WARNING_PREFIX: &str =
    "Possibly planted project folder not loaded: ";

/// The planted folders not confirmed yet (canonical paths). Always empty
/// off Windows.
pub fn planted_project_metadata() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        match codex_windows_sandbox::planted_project_metadata() {
            Ok(planted) => planted,
            Err(err) => {
                tracing::warn!("could not read the planted project metadata records: {err:#}");
                Vec::new()
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        Vec::new()
    }
}

/// Whether `path` is one of `planted`, compared canonically and, on
/// Windows, ignoring case.
pub fn is_planted_project_metadata(path: &Path, planted: &[PathBuf]) -> bool {
    if planted.is_empty() {
        return false;
    }
    let path = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    planted.iter().any(|entry| same_path(entry, &path))
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
    } else {
        a == b
    }
}

/// Why a planted `.codex` or `.agents` folder isn't loaded, and how to load it.
pub fn planted_project_metadata_reason(folder: &Path) -> String {
    format!(
        "{PLANTED_PROJECT_METADATA_WARNING_PREFIX}{}. It appeared while a non-admin Windows sandbox command ran, so a command may have planted it; its config, hooks and skills are ignored. Check its contents, then open Corbanu Terminal in this folder and choose to load it.",
        folder.display()
    )
}

#[cfg(test)]
#[path = "planted_project_metadata_tests.rs"]
mod tests;
