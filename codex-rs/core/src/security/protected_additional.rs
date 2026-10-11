//! Issue #245: a command's additional permissions never reopen a protected
//! path. The sandbox gives the most specific entry precedence, so an extra
//! read of `~/.ssh/id_ed25519` would override the PF-23 denial of `~/.ssh`
//! wherever the request was approved (a person, Guardian, a hook, the
//! approval cache, or a session or turn grant).
//!
//! Under Moderate and Aggressive, wherever the protected-path rules apply to
//! a run, a request whose additional permissions open a protected path is
//! refused and explained. Where Moderate lets a person's approval lift the
//! rules for one command (issue #239), nothing changes. Permissive has no
//! rules and is unchanged.

use std::path::Path;

use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::FileSystemPermissions;
use codex_protocol::permissions::FileSystemAccessMode;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_security_policy::SecurityLevel;

use super::protected_surface::ReadDenials;

/// The first protected path (as an absolute path) that an entry of
/// `additional` would make readable or writable. Deny entries only narrow.
pub(crate) fn opened_protected_path(
    denials: &ReadDenials,
    additional: &AdditionalPermissionProfile,
    cwd: &Path,
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
            FileSystemSandboxPolicy::restricted(vec![entry.clone()])
                .get_readable_roots_with_cwd(cwd)
        })
        .find(|root| matcher.is_read_denied(root.as_path()))
        .map(|root| root.display().to_string())
}

/// `additional` without the entries that would open a protected path, for
/// file tools that carry a session or turn grant into every call.
pub(crate) fn without_protected_paths(
    denials: &ReadDenials,
    additional: Option<AdditionalPermissionProfile>,
    cwd: &Path,
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
            opened_protected_path(denials, &alone, cwd).is_none()
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

/// File tools carry the session's and turn's granted permissions into every
/// call: once the protected-path rules apply (Moderate, Aggressive, or after
/// untrusted content), entries that would open a protected path are dropped.
/// No approval lifts the rules for file tools.
pub(crate) fn file_tool_grants(
    session: &crate::session::session::Session,
    turn: &crate::session::turn_context::TurnContext,
    workspace_roots: &[codex_utils_absolute_path::AbsolutePathBuf],
    additional: Option<AdditionalPermissionProfile>,
) -> Option<AdditionalPermissionProfile> {
    additional.as_ref()?;
    if session
        .services
        .model_client()
        .post_taint_state()
        .is_none_or(|state| !state.protected_paths_apply())
    {
        return additional;
    }
    #[allow(deprecated)]
    let turn_cwd = turn.cwd.clone();
    let denials = ReadDenials::for_turn(
        turn.config.codex_home.as_path(),
        &turn_cwd,
        workspace_roots,
        turn.config.permissions.permission_profile(),
    );
    without_protected_paths(&denials, additional, turn_cwd.as_path())
}

/// The refusal for an additional permission that would open `path`, in the
/// words #442 uses for a protected read under Aggressive.
pub(crate) fn refusal(level: SecurityLevel, path: &str) -> String {
    match level {
        SecurityLevel::Aggressive => super::protected_reads::explanation(path),
        _ => {
            let path = super::protected_reads::shown_path(path);
            format!(
                "Security level Moderate blocked access to `{path}`: an additional permission \
                 can never open protected files (credential files such as .env, SSH, cloud and \
                 CLI tokens, and Corbanu's sign-in, vault, config and history). To allow it for \
                 one command, a person can approve that command when it asks to run outside the \
                 sandbox, or switch to Permissive in /security (it applies after a restart)."
            )
        }
    }
}

#[cfg(test)]
#[path = "protected_additional_tests.rs"]
mod tests;
