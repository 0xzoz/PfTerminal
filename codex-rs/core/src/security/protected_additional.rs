//! Issue #245: a command's additional permissions never reopen a protected
//! path. The sandbox gives the most specific entry precedence, so an extra
//! read of `~/.ssh/id_ed25519` would override the PF-23 denial of `~/.ssh`
//! wherever the request was approved (a person, Guardian, a hook, the
//! approval cache, or a session or turn grant).
//!
//! Under Moderate and Aggressive, wherever the protected-path rules apply to
//! a run, a request whose additional permissions open a protected path is
//! refused and explained, and a permission grant (`request_permissions`)
//! never carries such entries into later commands or file tools. Where
//! Moderate lets a person's approval lift the rules for one command (issue
//! #239), nothing changes for reads. Permissive has no rules and is
//! unchanged.
//!
//! Issue #513: the same for the PF-23-S02 write protections. A write entry
//! equal to a read-only path wins the tie, a narrower one wins outright, and
//! a broader one opens a path the profile itself could not write (the
//! read-only rule is only added where it could). So additional write
//! permissions never make writable:
//! - the Corbanu home's runtime entries (`tmp` holds the sandbox launchers),
//!   under Moderate and Aggressive, even where an approval lifted the other
//!   rules, and never with a grant (#442);
//! - under Aggressive, the other read-only paths (start-up files, git hooks
//!   and config, the project's `.codex` and `.agents`) unless an Aggressive
//!   grant for the run lifted them.
//!
//! Moderate's other write protections are unchanged here (#513 records that
//! decision for security sign-off).

use std::path::Path;

use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::FileSystemPermissions;
use codex_protocol::permissions::FileSystemAccessMode;
use codex_protocol::permissions::FileSystemPath;
use codex_protocol::permissions::FileSystemSandboxEntry;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_security_policy::SecurityLevel;
use codex_utils_absolute_path::AbsolutePathBuf;

use super::protected_surface::ReadDenials;

/// Issue #513: the read-only paths additional write permissions may not
/// reopen in a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadOnlyScope {
    /// Only the Corbanu home's runtime entries: Moderate, and wherever an
    /// approval lifted the other rules for the run.
    Runtime,
    /// Every read-only path the denials keep (Aggressive; a grant for the
    /// run has already lifted what it lifts).
    Kept,
}

impl ReadOnlyScope {
    /// The scope once the run's rules are known.
    pub(crate) fn for_level(level: SecurityLevel) -> Self {
        match level {
            SecurityLevel::Aggressive => Self::Kept,
            _ => Self::Runtime,
        }
    }
}

/// What an additional permission would open, and so why it is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Opened {
    /// A protected path it would make readable.
    Read(String),
    /// A read-only path it would make writable. `grantable`: an Aggressive
    /// grant for the command would lift this protection (#442).
    Write { path: String, grantable: bool },
}

impl Opened {
    /// The refusal shown to the person and the model.
    pub(crate) fn refusal(&self, level: SecurityLevel) -> String {
        match self {
            Self::Read(path) => refusal(level, path),
            Self::Write { path, grantable } => write_refusal(level, path, *grantable),
        }
    }
}

/// What `additional` may not open in a run.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Check<'a> {
    pub(crate) denials: &'a ReadDenials,
    /// The run's file system policy with the rules applied, for writes above
    /// a read-only path. `None` when unknown: any such write is refused.
    pub(crate) protected: Option<&'a FileSystemSandboxPolicy>,
    /// Whether reads of protected paths are refused (not where an approval
    /// lifted the rules for the run, #239).
    pub(crate) reads: bool,
    pub(crate) writes: ReadOnlyScope,
}

/// The first protected path (as an absolute path) that an entry of
/// `additional` would make readable or writable, with relative and
/// project-root entries resolved against each of `cwds` (the folders the
/// sandbox may resolve them against). Deny entries only narrow.
pub(crate) fn opened_protected_path(
    denials: &ReadDenials,
    additional: &AdditionalPermissionProfile,
    cwds: &[&Path],
) -> Option<String> {
    let entries = &additional.file_system.as_ref()?.entries;
    if entries.is_empty() {
        return None;
    }
    let matcher = denials.matcher()?;
    entries
        .iter()
        .filter(|entry| entry.access != FileSystemAccessMode::Deny)
        // One entry at a time: an entry for `/` must not hide a narrower one.
        .flat_map(|entry| {
            let alone = FileSystemSandboxPolicy::restricted(vec![entry.clone()]);
            cwds.iter()
                .flat_map(move |cwd| alone.get_readable_roots_with_cwd(cwd))
        })
        .find(|root| matcher.is_read_denied(root.as_path()))
        .map(|root| root.display().to_string())
}

/// Issue #513: the first read-only path in `scope` that a write entry of
/// `additional` would make writable, resolved against each of `cwds`.
pub(crate) fn opened_read_only_path(
    denials: &ReadDenials,
    protected: Option<&FileSystemSandboxPolicy>,
    additional: &AdditionalPermissionProfile,
    cwds: &[&Path],
    scope: ReadOnlyScope,
) -> Option<AbsolutePathBuf> {
    let entries = &additional.file_system.as_ref()?.entries;
    let paths: Vec<&AbsolutePathBuf> = match scope {
        ReadOnlyScope::Runtime => denials.runtime_read_only_paths().collect(),
        ReadOnlyScope::Kept => denials.read_only_paths().collect(),
    };
    if paths.is_empty() {
        return None;
    }
    entries
        .iter()
        .filter(|entry| entry.access.can_write())
        .flat_map(|entry| cwds.iter().map(move |cwd| (entry, *cwd)))
        .find_map(|(entry, cwd)| reopened(&paths, protected, entry, cwd))
}

/// The read-only path of `paths` that the write `entry` reopens: one at or
/// above the paths it can write (equal entries tie in its favour, narrower
/// ones win), or one below them that `protected` with the entry added could
/// write. Links are followed as the sandbox does.
fn reopened(
    paths: &[&AbsolutePathBuf],
    protected: Option<&FileSystemSandboxPolicy>,
    entry: &FileSystemSandboxEntry,
    cwd: &Path,
) -> Option<AbsolutePathBuf> {
    let alone = FileSystemSandboxPolicy::restricted(vec![entry.clone()]);
    let roots: Vec<AbsolutePathBuf> = if alone.has_full_disk_write_access() {
        let root = cwd.ancestors().last()?;
        vec![AbsolutePathBuf::from_absolute_path(root).ok()?]
    } else {
        alone
            .get_writable_roots_with_cwd(cwd)
            .into_iter()
            .map(|writable| writable.root)
            .collect()
    };
    let mut spellings = roots.clone();
    for root in &roots {
        if let Some(real) = ReadDenials::real_location(root)
            && !spellings.contains(&real)
        {
            spellings.push(real);
        }
    }
    let merged = protected.map(|protected| {
        let mut merged = protected.clone();
        merged.entries.extend(spellings.iter().map(|root| {
            FileSystemSandboxEntry::new(
                FileSystemPath::Path { path: root.clone() },
                FileSystemAccessMode::Write,
            )
        }));
        merged
    });
    paths
        .iter()
        .find(|path| {
            spellings
                .iter()
                .any(|root| root.as_path().starts_with(path.as_path()))
                || match &merged {
                    Some(merged) => merged.can_write_path_with_cwd(path.as_path(), cwd),
                    None => spellings
                        .iter()
                        .any(|root| path.as_path().starts_with(root.as_path())),
                }
        })
        .map(|path| (*path).clone())
}

/// Issues #245 and #513: what `additional` would open under `check`.
pub(crate) fn opened(
    check: Check<'_>,
    additional: &AdditionalPermissionProfile,
    cwds: &[&Path],
) -> Option<Opened> {
    if check.reads
        && let Some(path) = opened_protected_path(check.denials, additional, cwds)
    {
        return Some(Opened::Read(path));
    }
    let path = opened_read_only_path(
        check.denials,
        check.protected,
        additional,
        cwds,
        check.writes,
    )?;
    let grantable = !check.denials.never_lifted_paths().any(|kept| *kept == path);
    Some(Opened::Write {
        path: path.display().to_string(),
        grantable,
    })
}

/// `additional` without the entries `check` refuses (for a grant, which
/// every later command carries).
pub(crate) fn without_opened(
    check: Check<'_>,
    additional: Option<AdditionalPermissionProfile>,
    cwds: &[&Path],
) -> Option<AdditionalPermissionProfile> {
    let mut additional = additional?;
    let Some(file_system) = additional.file_system.take() else {
        return Some(additional);
    };
    let FileSystemPermissions {
        entries,
        glob_scan_max_depth,
    } = file_system;
    let entries: Vec<_> = entries
        .into_iter()
        .filter(|entry| {
            let alone = AdditionalPermissionProfile {
                network: None,
                file_system: Some(FileSystemPermissions {
                    entries: vec![entry.clone()],
                    glob_scan_max_depth: None,
                }),
            };
            opened(check, &alone, cwds).is_none()
        })
        .collect();
    if !entries.is_empty() {
        additional.file_system = Some(FileSystemPermissions {
            entries,
            glob_scan_max_depth,
        });
    }
    (!additional.is_empty()).then_some(additional)
}

/// `additional` without the entries that would open a protected path to
/// reads (#245).
#[cfg(test)]
pub(crate) fn without_protected_paths(
    denials: &ReadDenials,
    additional: Option<AdditionalPermissionProfile>,
    cwds: &[&Path],
) -> Option<AdditionalPermissionProfile> {
    let check = Check {
        denials,
        protected: None,
        reads: true,
        writes: ReadOnlyScope::Runtime,
    };
    without_opened(check, additional, cwds)
}

/// Issues #245 and #513: the level whose protected-path rules apply to the
/// session now, if any (Moderate, Aggressive).
pub(crate) fn protected_level(session: &crate::session::session::Session) -> Option<SecurityLevel> {
    session
        .services
        .model_client()
        .post_taint_state()
        .filter(crate::security::tainted_action::PostTaintState::protected_paths_apply)
        .map(|state| state.level)
}

/// Issues #245 and #513: a permission grant (`request_permissions`)
/// without the entries that would open a protected path or reopen a
/// read-only one under `level`'s rules, and the refusal to show when any
/// were dropped. Every later command and file tool carries the grant, and
/// no Aggressive grant is known here, so under Aggressive every read-only
/// path counts. Each of `configs` (folder, permission profile) is checked;
/// an entry any of them refuses is dropped. Reads folders: call it off the
/// async thread ([`strip_grant_off_thread`]).
pub(crate) fn strip_grant(
    level: SecurityLevel,
    codex_home: &Path,
    configs: &[(AbsolutePathBuf, codex_protocol::models::PermissionProfile)],
    grant: AdditionalPermissionProfile,
) -> (Option<AdditionalPermissionProfile>, Option<String>) {
    let mut kept = Some(grant);
    let mut refusal = None;
    for (cwd, profile) in configs {
        let Some(grant) = kept.take() else {
            break;
        };
        let denials = ReadDenials::for_turn(codex_home, cwd, &[], profile);
        let protected = denials
            .apply(profile)
            .map(|protected| protected.file_system_sandbox_policy());
        let check = Check {
            denials: &denials,
            protected: protected.as_ref(),
            reads: true,
            writes: ReadOnlyScope::for_level(level),
        };
        let cwds = [cwd.as_path()];
        let Some(opened) = opened(check, &grant, &cwds) else {
            kept = Some(grant);
            continue;
        };
        refusal.get_or_insert_with(|| opened.refusal(level));
        kept = without_opened(check, Some(grant), &cwds);
    }
    if refusal.is_some() {
        tracing::warn!(
            target: "codex_core::security::tainted_action",
            "a permission grant would open a protected path; dropped from it"
        );
    }
    (kept, refusal)
}

/// [`strip_grant`] on a blocking thread (issue #524). If that fails the
/// grant is dropped (fail closed).
pub(crate) async fn strip_grant_off_thread(
    level: SecurityLevel,
    codex_home: std::path::PathBuf,
    configs: Vec<(AbsolutePathBuf, codex_protocol::models::PermissionProfile)>,
    grant: AdditionalPermissionProfile,
) -> (Option<AdditionalPermissionProfile>, Option<String>) {
    tokio::task::spawn_blocking(move || strip_grant(level, &codex_home, &configs, grant))
        .await
        .unwrap_or_else(|_| {
            tracing::warn!(
                target: "codex_core::security::tainted_action",
                "the permission grant could not be checked; dropped"
            );
            (None, None)
        })
}

/// The refusal for an additional permission that would open `path`, in the
/// words #442 uses for a protected read under Aggressive.
pub(crate) fn refusal(level: SecurityLevel, path: &str) -> String {
    match level {
        SecurityLevel::Aggressive => super::protected_reads::explanation(path),
        // Only Moderate reaches here: Permissive has no protected paths.
        _ => {
            let path = super::protected_reads::shown_path(path);
            format!(
                "Security level Moderate blocked access to `{path}`: an additional permission \
                 can never open protected files (credential files such as .env, SSH, cloud and \
                 CLI tokens, and Corbanu's sign-in, vault, config and history). To allow it, a \
                 person can switch to Permissive in /security (it applies after a restart)."
            )
        }
    }
}

/// Issue #513: the refusal for an additional permission that would make the
/// read-only `path` writable. `grantable`: an Aggressive grant for the
/// command lifts this protection.
pub(crate) fn write_refusal(level: SecurityLevel, path: &str, grantable: bool) -> String {
    let path = super::protected_reads::shown_path(path);
    let level = match level {
        SecurityLevel::Aggressive => "Aggressive",
        _ => "Moderate",
    };
    if grantable {
        format!(
            "Security level {level} blocked writing to `{path}`: files that run code or set \
             policy later (shell start-up files, git hooks and config, the project's .codex and \
             .agents) stay read-only, and an additional permission cannot change that. A person \
             can lift it for one command with \"Grant once\" when approving it, or switch to \
             Permissive in /security (it applies after a restart)."
        )
    } else {
        format!(
            "Security level {level} blocked writing to `{path}`: Corbanu's runtime files (the \
             sandbox helpers in its tmp folder, shell snapshots, skills and plugins) stay \
             read-only, and this cannot be granted, not even once. To allow it, a person can \
             switch to Permissive in /security (it applies after a restart)."
        )
    }
}

#[cfg(test)]
#[path = "protected_additional_tests.rs"]
mod tests;
