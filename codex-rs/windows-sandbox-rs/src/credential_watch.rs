//! #442 (Travis, 2026-10-12): on Windows too, a credential file that appears
//! while a sandboxed command runs stops that command, as on Linux.
//!
//! The sandbox denies reading the credential files that exist when a command
//! starts (deny ACEs on the files the policy's credential globs match). A
//! file created, renamed or moved in later gets the ACL of its folder, which
//! the command can read. So under a policy that denies the credential names
//! below a folder:
//! - the folder is watched (`ReadDirectoryChangesW`, recursive) from before
//!   the command starts until it ends;
//! - a credential file found at the start that the deny list does not hold
//!   (it appeared between the deny ACEs and the watch) refuses the command;
//! - a non-folder with a credential name created, renamed to or moved in
//!   (also inside a folder moved in), even at the place of a file that was
//!   denied (a new file has a new ACL), stops the command at once
//!   ([`CredentialWatch::found`]); later commands deny it;
//! - fail closed: a folder that cannot be watched refuses the command, and
//!   after lost events everything is scanned again (a file not on the deny
//!   list stops the command).
//!
//! Limits, as on Linux: the stop follows the event, so a process already
//! waiting for the file could read it in that moment; only the folders the
//! credential globs cover are watched; the file is left in place (its
//! creator is unknown). `.env.example`, `.env.sample` and `.env.template` are
//! not credential files.

use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::mpsc;

use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::credential_file_glob_root;
use codex_protocol::permissions::credential_file_names_shown;
use codex_protocol::permissions::credential_rule_roots;
use codex_protocol::permissions::is_credential_file_name;
use codex_protocol::permissions::is_credential_template_name;
use notify::EventKind;
use notify::RecursiveMode;
use notify::Watcher;
use notify::event::ModifyKind;
use notify::event::RenameMode;

/// Entries looked at by one scan, at most. Past it the command is refused.
const MAX_SCANNED_ENTRIES: usize = 1_000_000;

/// Called once, from the watch thread, when the command must stop.
pub type StopCommand = Box<dyn FnOnce() + Send>;

/// Watches the credential folders while one command runs. Dropping it ends
/// the watch.
pub struct CredentialWatch {
    found: Arc<Mutex<Option<String>>>,
    _watcher: notify::RecommendedWatcher,
}

impl CredentialWatch {
    /// Starts watching before the command starts. `None` when the policy
    /// denies no credential names below a folder. `known` are the files the
    /// deny list holds. `Err` is the refusal shown instead of running it.
    pub fn start(
        policy: &FileSystemSandboxPolicy,
        cwd: &Path,
        known: &[PathBuf],
        stop: StopCommand,
    ) -> Result<Option<Self>, String> {
        let roots = credential_watch_roots(policy, cwd);
        if roots.is_empty() {
            return Ok(None);
        }
        let names = Arc::new(Names::for_policy(policy, cwd));
        let known: Arc<HashSet<PathBuf>> = Arc::new(known.iter().map(|path| key(path)).collect());
        let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = tx.send(event);
        })
        .map_err(|err| refused_message(&format!("cannot watch for credential files: {err}")))?;
        for root in &roots {
            watcher
                .watch(root, RecursiveMode::Recursive)
                .map_err(|err| {
                    refused_message(&format!(
                        "cannot watch {} for credential files: {err}",
                        root.display()
                    ))
                })?;
        }
        // After the watch: anything created from here on raises an event.
        for root in &roots {
            if let Some(found) = scan(root, &known, &names)? {
                return Err(refused_message(&format!(
                    "the credential file {found} appeared just before the command started; run \
                     it again"
                )));
            }
        }
        let found = Arc::new(Mutex::new(None));
        let thread_found = Arc::clone(&found);
        std::thread::Builder::new()
            .name("corbanu-credential-watch".to_string())
            .spawn(move || {
                // Ends when the watcher is dropped (the channel closes).
                for event in rx {
                    if let Some(path) = must_stop(event, &roots, &known, &names) {
                        *thread_found.lock().unwrap_or_else(PoisonError::into_inner) = Some(path);
                        stop();
                        return;
                    }
                }
            })
            .map_err(|err| refused_message(&format!("cannot watch for credential files: {err}")))?;
        Ok(Some(Self {
            found,
            _watcher: watcher,
        }))
    }

    /// The credential file whose appearance stopped the command, if any.
    pub fn found(&self) -> Option<String> {
        self.found
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The folders whose credential names the policy denies, existing, outermost
/// only. A drive root is left out (its glob is not expanded either).
pub fn credential_watch_roots(policy: &FileSystemSandboxPolicy, cwd: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = policy
        .get_unreadable_globs_with_cwd(cwd)
        .iter()
        .filter_map(|pattern| credential_file_glob_root(pattern))
        .filter(|root| root.parent().is_some())
        .filter_map(|root| dunce::canonicalize(root).ok())
        .filter(|root| root.parent().is_some() && root.is_dir())
        .collect();
    roots.sort();
    roots.dedup();
    roots
        .iter()
        .filter(|root| {
            !roots
                .iter()
                .any(|other| other != *root && root.starts_with(other))
        })
        .cloned()
        .collect()
}

/// What to stop for, if anything, after one watch event.
fn must_stop(
    event: notify::Result<notify::Event>,
    roots: &[PathBuf],
    known: &HashSet<PathBuf>,
    names: &Names,
) -> Option<String> {
    let event = match event {
        Ok(event) => event,
        // Events may be lost: look at everything again.
        Err(_) => return rescan(roots, known, names),
    };
    if event.need_rescan() {
        return rescan(roots, known, names);
    }
    let appeared = matches!(
        event.kind,
        EventKind::Create(_)
            | EventKind::Any
            | EventKind::Modify(ModifyKind::Name(
                RenameMode::To | RenameMode::Both | RenameMode::Any
            ))
    );
    if !appeared {
        return None;
    }
    for path in &event.paths {
        match std::fs::symlink_metadata(path) {
            // A folder created or moved in: anything inside it is new.
            Ok(metadata) if metadata.is_dir() => {
                if let Some(found) = scan(path, &HashSet::new(), names).unwrap_or_else(Some) {
                    return Some(found);
                }
            }
            // A new file, even at the place of a denied one, has a new ACL.
            // One already gone may have been read.
            _ if names.is_credential(path) => {
                return Some(path.display().to_string());
            }
            _ => {}
        }
    }
    None
}

fn rescan(roots: &[PathBuf], known: &HashSet<PathBuf>, names: &Names) -> Option<String> {
    for root in roots {
        match scan(root, known, names) {
            Ok(None) => {}
            Ok(Some(found)) => return Some(found),
            Err(reason) => return Some(reason),
        }
    }
    None
}

/// A credential file below `folder` (links and junctions are not followed)
/// that `known` does not hold. `Err` when the folder cannot be checked.
fn scan(folder: &Path, known: &HashSet<PathBuf>, names: &Names) -> Result<Option<String>, String> {
    let mut pending = vec![folder.to_path_buf()];
    let mut seen = 0_usize;
    while let Some(folder) = pending.pop() {
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            // Removed meanwhile (a temporary build folder).
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(format!("cannot list {}: {err}", folder.display())),
        };
        for entry in entries {
            let entry = entry.map_err(|err| format!("cannot list {}: {err}", folder.display()))?;
            seen += 1;
            if seen > MAX_SCANNED_ENTRIES {
                return Err(format!(
                    "more than {MAX_SCANNED_ENTRIES} files to check for credential files below {}",
                    folder.display()
                ));
            }
            let path = entry.path();
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(format!("cannot read {}: {err}", path.display())),
            };
            if metadata.is_dir() && !is_link(&metadata) {
                pending.push(path);
            } else if !metadata.is_dir()
                && names.is_credential(&path)
                && !known.contains(&key(&path))
            {
                return Ok(Some(path.display().to_string()));
            }
        }
    }
    Ok(None)
}

/// Which file names count as credential files where.
struct Names {
    /// Below these the whole credential rule applies, so the templates are
    /// not credential files; below a folder only a person's own `**/.env.*`
    /// covers, they are.
    template_roots: Vec<PathBuf>,
}

impl Names {
    fn for_policy(policy: &FileSystemSandboxPolicy, cwd: &Path) -> Self {
        Self {
            template_roots: credential_rule_roots(&policy.get_unreadable_globs_with_cwd(cwd))
                .into_iter()
                .filter_map(|root| dunce::canonicalize(root).ok())
                .map(|root| key(&root))
                .collect(),
        }
    }

    fn is_credential(&self, path: &Path) -> bool {
        let Some(name) = path.file_name() else {
            return false;
        };
        is_credential_file_name(name)
            || (is_credential_template_name(name)
                && !self
                    .template_roots
                    .iter()
                    .any(|root| key(path).starts_with(root)))
    }
}

#[cfg(windows)]
fn is_link(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_link(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

/// How a path is compared: Windows paths match in any letter case.
fn key(path: &Path) -> PathBuf {
    let path = dunce::simplified(path);
    if cfg!(windows) {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    } else {
        path.to_path_buf()
    }
}

/// What the person and the model see when a command is refused.
pub fn refused_message(reason: &str) -> String {
    format!(
        "Operation not permitted: the sandbox refuses to run this command because it cannot keep \
         credential files ({}) hidden from it: {reason}.",
        credential_file_names_shown()
    )
}

/// What the person and the model see when a command was stopped.
pub fn stopped_message(found: &str) -> String {
    format!(
        "Operation not permitted: the sandbox stopped this command because a credential file \
         ({found}) appeared while it ran. Files named {} are hidden from agent commands; one \
         created after a command starts cannot be hidden from it, so the command is stopped. \
         Later commands cannot read it.",
        credential_file_names_shown()
    )
}

#[cfg(test)]
#[path = "credential_watch_tests.rs"]
mod tests;
