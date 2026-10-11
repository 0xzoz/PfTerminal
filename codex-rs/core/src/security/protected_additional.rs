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
//! #239), nothing changes. Permissive has no rules and is unchanged.

use std::path::Path;

use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::FileSystemPermissions;
use codex_protocol::permissions::FileSystemAccessMode;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_security_policy::SecurityLevel;

use super::protected_surface::ReadDenials;

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

/// `additional` without the entries that would open a protected path (for a
/// grant, which every later command carries).
pub(crate) fn without_protected_paths(
    denials: &ReadDenials,
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
            opened_protected_path(denials, &alone, cwds).is_none()
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

/// Issue #245: a permission grant (`request_permissions`) without the
/// entries that would open a protected path, while the protected-path rules
/// apply (Moderate, Aggressive), and the refusal to show when any were
/// dropped. Every later command and file tool carries the grant. `cwd` is
/// the folder it is resolved against.
pub(crate) fn strip_grant(
    session: &crate::session::session::Session,
    config: &crate::config::Config,
    cwd: &codex_utils_absolute_path::AbsolutePathBuf,
    grant: AdditionalPermissionProfile,
) -> (Option<AdditionalPermissionProfile>, Option<String>) {
    let Some(state) = session
        .services
        .model_client()
        .post_taint_state()
        .filter(crate::security::tainted_action::PostTaintState::protected_paths_apply)
    else {
        return (Some(grant), None);
    };
    let denials = ReadDenials::for_turn(
        config.codex_home.as_path(),
        cwd,
        &[],
        config.permissions.permission_profile(),
    );
    let Some(path) = opened_protected_path(&denials, &grant, &[cwd.as_path()]) else {
        return (Some(grant), None);
    };
    tracing::warn!(
        target: "codex_core::security::tainted_action",
        "a permission grant would open a protected path; dropped from it"
    );
    (
        without_protected_paths(&denials, Some(grant), &[cwd.as_path()]),
        Some(refusal(state.level, &path)),
    )
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

#[cfg(test)]
#[path = "protected_additional_tests.rs"]
mod tests;
