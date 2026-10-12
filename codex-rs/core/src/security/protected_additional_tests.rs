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
        &codex_home.join("tmp"),
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
                &[f.project.as_path()]
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
        opened_protected_path(&f.denials, &with_root, &[f.project.as_path()]),
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
                &[f.project.as_path()]
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
        opened_protected_path(&f.denials, &network_only, &[f.project.as_path()]),
        None
    );
}

#[test]
fn issue_245_relative_and_project_root_entries_resolve_against_every_folder() {
    let f = fixture();
    let project_root_key = additional(vec![FileSystemSandboxEntry::new(
        FileSystemPath::Special {
            value: FileSystemSpecialPath::project_roots(Some(".ssh/id_ed25519".into())),
        },
        FileSystemAccessMode::Read,
    )]);
    // Checked against the project only it opens nothing; the sandbox may
    // resolve it against the home folder, where it opens the key.
    assert_eq!(
        opened_protected_path(&f.denials, &project_root_key, &[f.project.as_path()]),
        None
    );
    assert_eq!(
        opened_protected_path(
            &f.denials,
            &project_root_key,
            &[f.project.as_path(), f.user_home.as_path()]
        ),
        Some(f.user_home.join(".ssh/id_ed25519").display().to_string())
    );
    // A write above a protected folder stays below its denial.
    assert_eq!(
        opened_protected_path(
            &f.denials,
            &additional(vec![entry(&f.user_home, FileSystemAccessMode::Write)]),
            &[f.project.as_path()]
        ),
        None
    );
}

#[test]
fn issue_245_grants_drop_only_protected_entries() {
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
            &[f.project.as_path()]
        ),
        Some(additional(vec![notes]))
    );
    assert_eq!(
        without_protected_paths(
            &f.denials,
            Some(additional(vec![key.clone()])),
            &[f.project.as_path()]
        ),
        None
    );
    let mut with_network = additional(vec![key]);
    with_network.network = Some(NetworkPermissions {
        enabled: Some(true),
    });
    assert_eq!(
        without_protected_paths(&f.denials, Some(with_network), &[f.project.as_path()]),
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

/// The run's policy with the rules applied, for a profile that reads
/// everything and writes `writable`.
fn protected_policy(
    denials: &ReadDenials,
    writable: &[&std::path::Path],
) -> FileSystemSandboxPolicy {
    let mut entries = vec![FileSystemSandboxEntry::new(
        FileSystemPath::Special {
            value: FileSystemSpecialPath::Root,
        },
        FileSystemAccessMode::Read,
    )];
    entries.extend(
        writable
            .iter()
            .map(|path| entry(path, FileSystemAccessMode::Write)),
    );
    let profile = codex_protocol::models::PermissionProfile::from_runtime_permissions(
        &FileSystemSandboxPolicy::restricted(entries),
        codex_protocol::permissions::NetworkSandboxPolicy::Restricted,
    );
    denials
        .apply(&profile)
        .expect("restricted profile")
        .file_system_sandbox_policy()
}

fn write(path: &std::path::Path) -> AdditionalPermissionProfile {
    additional(vec![entry(path, FileSystemAccessMode::Write)])
}

#[test]
fn issue_513_a_write_never_reopens_the_runtime_entries() {
    let f = fixture();
    let runtime = f.codex_home.join("tmp");
    let launcher = runtime.join("launcher");
    let link = f.project.join("helpers");
    std::os::unix::fs::symlink(&runtime, &link).unwrap();
    let protected = protected_policy(&f.denials, &[f.project.as_path()]);
    let root = additional(vec![FileSystemSandboxEntry::new(
        FileSystemPath::Special {
            value: FileSystemSpecialPath::Root,
        },
        FileSystemAccessMode::Write,
    )]);
    for (name, request) in [
        ("equal", write(&runtime)),
        ("narrower", write(&launcher)),
        // Above it, where the profile itself could not write it.
        ("broader", write(&f.codex_home)),
        ("root", root),
        ("through a link", write(&link.join("launcher"))),
    ] {
        for scope in [ReadOnlyScope::Runtime, ReadOnlyScope::Kept] {
            assert!(
                opened_read_only_path(
                    &f.denials,
                    Some(&protected),
                    &request,
                    &[f.project.as_path()],
                    scope
                )
                .is_some(),
                "{name} {scope:?}"
            );
        }
    }
    // Reads never write; the rule holds where the profile could write.
    let read = additional(vec![entry(&runtime, FileSystemAccessMode::Read)]);
    let writes_home = protected_policy(&f.denials, &[f.project.as_path(), f.codex_home.as_path()]);
    for (name, request, protected) in [
        ("read", read, &protected),
        (
            "broader, profile writes it",
            write(&f.codex_home),
            &writes_home,
        ),
        ("project", write(&f.project.join("notes.txt")), &protected),
    ] {
        assert_eq!(
            opened_read_only_path(
                &f.denials,
                Some(protected),
                &request,
                &[f.project.as_path()],
                ReadOnlyScope::Kept
            ),
            None,
            "{name}"
        );
    }
}

#[test]
fn issue_513_other_write_protections_count_under_aggressive_until_a_grant_lifts_them() {
    let f = fixture();
    let agents = f.project.join(".agents");
    let protected = protected_policy(&f.denials, &[f.project.as_path()]);
    let check = |denials: &ReadDenials, writes| {
        opened(
            Check {
                denials,
                protected: Some(&protected),
                reads: true,
                writes,
            },
            &write(&agents.join("skill.md")),
            &[f.project.as_path()],
        )
    };
    assert_eq!(
        check(&f.denials, ReadOnlyScope::Kept),
        Some(Opened::Write {
            path: agents.display().to_string(),
            grantable: true,
        })
    );
    // Moderate: unchanged (#513 decision).
    assert_eq!(check(&f.denials, ReadOnlyScope::Runtime), None);
    // An Aggressive grant lifts it; never the runtime entries.
    let mut granted = ReadDenials::collect(
        &f.codex_home,
        Some(&f.user_home),
        /*claude_config_dir*/ None,
        &[AbsolutePathBuf::from_absolute_path(&f.project).unwrap()],
    );
    granted.lift_write_rules();
    assert_eq!(check(&granted, ReadOnlyScope::Kept), None);
    let runtime = f.codex_home.join("tmp");
    assert_eq!(
        opened(
            Check {
                denials: &granted,
                protected: Some(&protected),
                reads: true,
                writes: ReadOnlyScope::Kept,
            },
            &write(&runtime),
            &[f.project.as_path()],
        ),
        Some(Opened::Write {
            path: runtime.display().to_string(),
            grantable: false,
        })
    );
}

#[test]
fn issue_513_grants_drop_entries_that_reopen_a_write_protection() {
    let f = fixture();
    let notes = entry(&f.project.join("notes.txt"), FileSystemAccessMode::Write);
    let launcher = entry(
        &f.codex_home.join("tmp/launcher"),
        FileSystemAccessMode::Write,
    );
    let protected = protected_policy(&f.denials, &[f.project.as_path()]);
    let check = Check {
        denials: &f.denials,
        protected: Some(&protected),
        reads: true,
        writes: ReadOnlyScope::Runtime,
    };
    assert_eq!(
        without_opened(
            check,
            Some(additional(vec![notes.clone(), launcher])),
            &[f.project.as_path()]
        ),
        Some(additional(vec![notes]))
    );
}

#[test]
fn issue_513_write_refusals_say_whether_a_grant_can_lift_them() {
    let runtime = write_refusal(
        SecurityLevel::Moderate,
        "/home/u/.corbanu/tmp\u{1b}[2J",
        false,
    );
    assert!(
        runtime.starts_with("Security level Moderate blocked writing to `/home/u/.corbanu/tmp"),
        "{runtime}"
    );
    assert!(!runtime.contains('\u{1b}'), "{runtime}");
    assert!(runtime.contains("cannot be granted"), "{runtime}");
    assert!(runtime.contains("Permissive in /security"), "{runtime}");
    let hook = write_refusal(SecurityLevel::Aggressive, "/p/.git/hooks", true);
    assert!(hook.starts_with("Security level Aggressive"), "{hook}");
    assert!(hook.contains("\"Grant once\""), "{hook}");
    assert!(!hook.contains("cannot be granted"), "{hook}");
    assert_eq!(
        ReadOnlyScope::for_level(SecurityLevel::Aggressive),
        ReadOnlyScope::Kept
    );
    assert_eq!(
        ReadOnlyScope::for_level(SecurityLevel::Moderate),
        ReadOnlyScope::Runtime
    );
}

/// Review r2 (#524): a grant is checked with the workspace roots, so a write
/// to a second root's `.agents` is dropped under Aggressive (it would make
/// every later command fail) and kept under Moderate (unchanged there).
#[test]
fn issue_524_grants_are_checked_with_every_workspace_root() {
    let f = fixture();
    let second = f.user_home.join("second");
    std::fs::create_dir_all(&second).unwrap();
    let roots: Vec<AbsolutePathBuf> = [&f.project, &second]
        .iter()
        .map(|root| AbsolutePathBuf::from_absolute_path(root.as_path()).unwrap())
        .collect();
    let profile = codex_protocol::models::PermissionProfile::from_runtime_permissions(
        &FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry::new(
                FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                FileSystemAccessMode::Read,
            ),
            entry(&f.project, FileSystemAccessMode::Write),
            entry(&second, FileSystemAccessMode::Write),
        ]),
        codex_protocol::permissions::NetworkSandboxPolicy::Restricted,
    );
    let configs = [GrantCheckConfig {
        cwd: roots[0].clone(),
        workspace_roots: roots.clone(),
        profile,
    }];
    let grant = write(&second.join(".agents/skill.md"));
    let (kept, refusal) = strip_grant(
        SecurityLevel::Aggressive,
        &f.codex_home,
        &configs,
        grant.clone(),
    );
    assert_eq!(kept, None);
    let refusal = refusal.expect("a refusal");
    assert!(refusal.contains("blocked writing to"), "{refusal}");
    assert_eq!(
        strip_grant(
            SecurityLevel::Moderate,
            &f.codex_home,
            &configs,
            grant.clone()
        ),
        (Some(grant), None)
    );
}

#[test]
fn issue_524_a_failed_check_is_refused_and_explained() {
    let text = Opened::Unchecked.refusal(SecurityLevel::Aggressive);
    assert_eq!(text, unchecked_refusal(SecurityLevel::Aggressive));
    assert!(
        text.starts_with("Security level Aggressive could not check"),
        "{text}"
    );
    assert!(text.contains("Permissive in /security"), "{text}");
}
