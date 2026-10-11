#![cfg(unix)]

use super::*;
use codex_protocol::models::NetworkPermissions;
use codex_protocol::permissions::FileSystemPath;
use codex_protocol::permissions::FileSystemSandboxEntry;
use codex_protocol::permissions::FileSystemSpecialPath;
use codex_utils_absolute_path::AbsolutePathBuf;
use pretty_assertions::assert_eq;
use std::path::PathBuf;

struct Fixture {
    _dir: tempfile::TempDir,
    user_home: PathBuf,
    codex_home: PathBuf,
    project: PathBuf,
    denials: ReadDenials,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let user_home = dir.path().canonicalize().unwrap().join("user");
    let codex_home = user_home.join(".corbanu");
    let project = user_home.join("project");
    for folder in [
        &codex_home.join("wallet"),
        &user_home.join(".ssh"),
        &project,
    ] {
        std::fs::create_dir_all(folder).unwrap();
    }
    std::fs::write(user_home.join(".ssh/id_ed25519"), "x").unwrap();
    std::fs::write(codex_home.join("wallet/dummy.txt"), "x").unwrap();
    std::fs::write(codex_home.join("state_5.sqlite"), "x").unwrap();
    std::fs::write(project.join("notes.txt"), "x").unwrap();
    let denials = ReadDenials::collect(
        &codex_home,
        Some(&user_home),
        /*claude_config_dir*/ None,
        &[AbsolutePathBuf::from_absolute_path(&project).unwrap()],
    );
    Fixture {
        _dir: dir,
        user_home,
        codex_home,
        project,
        denials,
    }
}

fn entry(path: &std::path::Path, access: FileSystemAccessMode) -> FileSystemSandboxEntry {
    FileSystemSandboxEntry::new(
        FileSystemPath::Path {
            path: AbsolutePathBuf::from_absolute_path(path).unwrap(),
        },
        access,
    )
}

fn additional(entries: Vec<FileSystemSandboxEntry>) -> AdditionalPermissionProfile {
    AdditionalPermissionProfile {
        network: None,
        file_system: Some(FileSystemPermissions {
            entries,
            glob_scan_max_depth: None,
        }),
    }
}

#[test]
fn issue_245_an_entry_at_or_under_a_protected_path_opens_it() {
    let f = fixture();
    let ssh_key = f.user_home.join(".ssh/id_ed25519");
    let wallet_file = f.codex_home.join("wallet/dummy.txt");
    let database = f.codex_home.join("state_5.sqlite");
    let link = f.project.join("keys");
    std::os::unix::fs::symlink(f.user_home.join(".ssh"), &link).unwrap();
    for (path, access) in [
        (ssh_key.clone(), FileSystemAccessMode::Read),
        (ssh_key.clone(), FileSystemAccessMode::Write),
        (f.user_home.join(".ssh"), FileSystemAccessMode::Read),
        (wallet_file, FileSystemAccessMode::Read),
        (database, FileSystemAccessMode::Read),
        // A link from the project into a protected folder.
        (link.join("id_ed25519"), FileSystemAccessMode::Read),
    ] {
        assert!(
            opened_protected_path(
                &f.denials,
                &additional(vec![entry(&path, access)]),
                &f.project
            )
            .is_some(),
            "{} {access:?}",
            path.display()
        );
    }
    // An entry for `/` does not hide a narrower one.
    let with_root = additional(vec![
        FileSystemSandboxEntry::new(
            FileSystemPath::Special {
                value: FileSystemSpecialPath::Root,
            },
            FileSystemAccessMode::Read,
        ),
        entry(&ssh_key, FileSystemAccessMode::Read),
    ]);
    assert_eq!(
        opened_protected_path(&f.denials, &with_root, &f.project),
        Some(ssh_key.display().to_string())
    );
}

#[test]
fn issue_245_entries_that_open_no_protected_path_pass() {
    let f = fixture();
    for (path, access) in [
        (f.project.join("notes.txt"), FileSystemAccessMode::Read),
        (f.project.clone(), FileSystemAccessMode::Write),
        // A folder above a protected path: the denial is more specific.
        (f.user_home.clone(), FileSystemAccessMode::Read),
        // Deny entries only narrow.
        (f.user_home.join(".ssh"), FileSystemAccessMode::Deny),
    ] {
        assert_eq!(
            opened_protected_path(
                &f.denials,
                &additional(vec![entry(&path, access)]),
                &f.project
            ),
            None,
            "{} {access:?}",
            path.display()
        );
    }
    let network_only = AdditionalPermissionProfile {
        network: Some(NetworkPermissions {
            enabled: Some(true),
        }),
        file_system: None,
    };
    assert_eq!(
        opened_protected_path(&f.denials, &network_only, &f.project),
        None
    );
}

#[test]
fn issue_245_file_tool_grants_drop_only_protected_entries() {
    let f = fixture();
    let notes = entry(&f.project.join("notes.txt"), FileSystemAccessMode::Read);
    let key = entry(
        &f.user_home.join(".ssh/id_ed25519"),
        FileSystemAccessMode::Read,
    );
    assert_eq!(
        without_protected_paths(
            &f.denials,
            Some(additional(vec![notes.clone(), key.clone()])),
            &f.project
        ),
        Some(additional(vec![notes]))
    );
    assert_eq!(
        without_protected_paths(&f.denials, Some(additional(vec![key.clone()])), &f.project),
        None
    );
    let mut with_network = additional(vec![key]);
    with_network.network = Some(NetworkPermissions {
        enabled: Some(true),
    });
    assert_eq!(
        without_protected_paths(&f.denials, Some(with_network), &f.project),
        Some(AdditionalPermissionProfile {
            network: Some(NetworkPermissions {
                enabled: Some(true)
            }),
            file_system: None,
        })
    );
}

#[test]
fn issue_245_refusals_name_the_level_and_the_way_forward() {
    assert_eq!(
        refusal(SecurityLevel::Aggressive, "/home/u/.ssh/id"),
        crate::security::protected_reads::explanation("/home/u/.ssh/id")
    );
    let moderate = refusal(SecurityLevel::Moderate, "/home/u/.ssh/id\u{1b}[2J");
    assert!(
        moderate.starts_with("Security level Moderate blocked access to `/home/u/.ssh/id"),
        "{moderate}"
    );
    assert!(!moderate.contains('\u{1b}'), "{moderate}");
    assert!(moderate.contains("protected files"), "{moderate}");
    assert!(moderate.contains("Permissive in /security"), "{moderate}");
}
