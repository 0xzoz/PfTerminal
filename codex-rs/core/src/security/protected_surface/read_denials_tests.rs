#![cfg(unix)]

use super::*;
use codex_protocol::permissions::NetworkSandboxPolicy;
use pretty_assertions::assert_eq;

fn abs(path: &Path) -> AbsolutePathBuf {
    AbsolutePathBuf::from_absolute_path(path).unwrap()
}

struct Fixture {
    _dir: tempfile::TempDir,
    user_home: std::path::PathBuf,
    codex_home: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let user_home = dir.path().canonicalize().unwrap().join("user");
    let codex_home = user_home.join(".corbanu");
    for folder in [
        "tmp/arg0",
        "shell_snapshots",
        "skills",
        "sessions",
        "worktrees/w",
    ] {
        std::fs::create_dir_all(codex_home.join(folder)).unwrap();
    }
    for file in ["auth.json", "config.toml", "notes.txt", "state_5.sqlite"] {
        std::fs::write(codex_home.join(file), "x").unwrap();
    }
    std::fs::create_dir_all(user_home.join(".ssh")).unwrap();
    std::fs::write(user_home.join(".ssh/id_ed25519"), "x").unwrap();
    std::fs::create_dir_all(user_home.join("project")).unwrap();
    Fixture {
        _dir: dir,
        user_home,
        codex_home,
    }
}

fn workspace(cwd: &Path) -> PermissionProfile {
    PermissionProfile::workspace_write().materialize_project_roots_with_workspace_roots(&[abs(cwd)])
}

#[test]
fn pf_23_s01_corbanu_home_is_denied_by_default_and_runtime_entries_stay() {
    let fx = fixture();
    let cwd = fx.user_home.join("project");
    let denials = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)]);
    let profile = denials.apply(&workspace(&cwd)).unwrap();
    let policy = profile.file_system_sandbox_policy();
    let readable = |path: &Path| policy.can_read_path_with_cwd(path, &cwd);
    for denied in [
        "auth.json",
        "config.toml",
        "notes.txt",
        "sessions",
        "secrets",
        "source-origin.key",
        "memories",
    ] {
        assert!(!readable(&fx.codex_home.join(denied)), "{denied}");
    }
    for kept in ["tmp/arg0", "shell_snapshots", "skills", "worktrees/w"] {
        assert!(readable(&fx.codex_home.join(kept)), "{kept}");
    }
    for denied in [
        ".ssh/id_ed25519",
        ".aws/credentials",
        ".codex",
        ".config/solana",
    ] {
        assert!(!readable(&fx.user_home.join(denied)), "{denied}");
    }
    assert!(readable(&cwd.join("src/main.rs")));
    assert_eq!(
        policy.get_unreadable_globs_with_cwd(&cwd),
        vec![
            fx.codex_home
                .join("*.sqlite*")
                .to_string_lossy()
                .into_owned()
        ]
    );
    // Denials can never be dropped by running the command unsandboxed.
    assert!(policy.has_denied_read_restrictions());
}

#[test]
fn pf_23_s01_full_access_gets_a_sandbox_that_only_denies_reads() {
    let fx = fixture();
    let cwd = fx.user_home.join("project");
    let denials = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)]);
    let profile = denials.apply(&PermissionProfile::Disabled).unwrap();
    let policy = profile.file_system_sandbox_policy();
    assert_eq!(policy.kind, FileSystemSandboxKind::Restricted);
    assert!(!policy.can_read_path_with_cwd(&fx.user_home.join(".ssh/id_ed25519"), &cwd));
    assert!(policy.can_write_path_with_cwd(Path::new("/usr/local/anything"), &cwd));
    assert_eq!(
        profile.network_sandbox_policy(),
        NetworkSandboxPolicy::Enabled
    );
    // An external sandbox cannot take the rules.
    assert_eq!(
        denials.apply(&PermissionProfile::External {
            network: NetworkSandboxPolicy::Restricted
        }),
        None
    );
}

#[test]
fn pf_23_s01_a_denial_holding_the_workspace_is_skipped() {
    let fx = fixture();
    // Working inside `~/.ssh` (or a home entry) must not deny the workspace.
    let cwd = fx.user_home.join(".ssh");
    let denials = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)]);
    assert!(denials.skipped.contains(&abs(&cwd)));
    assert!(denials.paths().all(|path| !cwd.starts_with(path.as_path())));
    // The working folder inside the Corbanu home skips only its own entry.
    let cwd = fx.codex_home.join("notes.txt");
    let denials = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)]);
    assert_eq!(denials.skipped, vec![abs(&cwd)]);
    assert!(
        denials
            .paths()
            .any(|path| path == &abs(&fx.codex_home.join("auth.json")))
    );
}

#[test]
fn pf_23_s01_claude_config_dir_credentials_are_denied() {
    let fx = fixture();
    let cwd = fx.user_home.join("project");
    let claude = fx.user_home.join("claude-config");
    let denials = ReadDenials::collect(
        &fx.codex_home,
        Some(&fx.user_home),
        Some(&claude),
        &[abs(&cwd)],
    );
    assert!(
        denials
            .paths()
            .any(|path| path == &abs(&claude.join(".credentials.json")))
    );
}

/// Review 1: in-process file tools (patch pre-check, structured edits, image
/// view, extension tools) get the same denials once the session is tainted.
/// Under a protected level (here Moderate) the denials also apply before taint
/// (issue #239): a workspace-write sandbox otherwise reads `$HOME` credentials.
#[tokio::test]
async fn pf_23_s01_file_tools_get_the_denials_after_taint() {
    let (session, turn) = crate::session::tests::make_session_and_context().await;
    let client = (*session.services.model_client())
        .clone()
        .with_ingress_level(codex_security_policy::SecurityLevel::Moderate)
        .with_source_envelopes(true);
    session.services.replace_model_client(client);
    let environment = turn
        .environments
        .primary()
        .expect("a primary environment")
        .clone();
    let auth = turn.config.codex_home.join("auth.json");
    let readable = |context: codex_file_system::FileSystemSandboxContext| {
        let profile = PermissionProfile::try_from(context.permissions).unwrap();
        #[allow(deprecated)]
        let cwd = turn.cwd.clone();
        profile
            .file_system_sandbox_policy()
            .can_read_path_with_cwd(auth.as_path(), cwd.as_path())
    };
    let context = || turn.file_system_sandbox_context(None, &environment);
    // Issue #239: under Moderate the denials apply before taint too.
    let before = protect_file_tool_context(&session, &turn, context());
    assert!(readable(context()));
    assert!(!readable(before));

    session
        .services
        .model_client()
        .note_unrecorded_input_for_taint();
    let after = protect_file_tool_context(&session, &turn, context());
    assert!(readable(context()));
    assert!(!readable(after));
}

/// Review 2: host reads of model-named files (Codex Apps uploads) follow the
/// same denials after taint, through symlinks too. Under a protected level
/// the policy is also active before taint (issue #239).
#[tokio::test]
async fn pf_23_s01_upload_reads_follow_the_denials_after_taint() {
    let (session, turn) = crate::session::tests::make_session_and_context().await;
    let client = (*session.services.model_client())
        .clone()
        .with_ingress_level(codex_security_policy::SecurityLevel::Moderate)
        .with_source_envelopes(true);
    session.services.replace_model_client(client);
    #[allow(deprecated)]
    let cwd = turn.cwd.clone();
    let home = turn.config.codex_home.to_path_buf();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("auth.json"), "x").unwrap();
    // Issue #239: under Moderate the upload read policy is active before taint.
    let before = post_taint_read_policy(&session, &turn).expect("protected level active");
    assert!(!readable_under(
        &before,
        &home.join("auth.json"),
        cwd.as_path()
    ));
    session
        .services
        .model_client()
        .note_unrecorded_input_for_taint();
    let policy = post_taint_read_policy(&session, &turn).expect("tainted");
    let outside = tempfile::tempdir().unwrap();
    let link = outside.path().join("innocent.txt");
    std::os::unix::fs::symlink(home.join("auth.json"), &link).unwrap();
    let plain = outside.path().join("plain.txt");
    std::fs::write(&plain, "x").unwrap();
    assert!(!readable_under(
        &policy,
        &home.join("auth.json"),
        cwd.as_path()
    ));
    assert!(!readable_under(&policy, &link, cwd.as_path()));
    assert!(readable_under(&policy, &plain, cwd.as_path()));
}

/// Issue #562: the Corbanu home's runtime entries stay read-only where they
/// are missing too (on Linux too: bubblewrap holds a missing one with a
/// placeholder), and `AGENTS.override.md` is denied before it exists.
#[test]
fn issue_562_missing_runtime_entries_stay_read_only() {
    let fx = fixture();
    let cwd = fx.user_home.join("project");
    for missing in [
        "plugins",
        "packages",
        ".tmp",
        "AGENTS.md",
        "AGENTS.override.md",
    ] {
        assert!(!fx.codex_home.join(missing).exists(), "{missing}");
    }
    let denials = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)]);
    let policy = denials
        .apply(&PermissionProfile::Disabled)
        .unwrap()
        .file_system_sandbox_policy();
    let writable = |path: &Path| policy.can_write_path_with_cwd(path, &cwd);
    let readable = |path: &Path| policy.can_read_path_with_cwd(path, &cwd);
    for runtime in [
        fx.codex_home.join("AGENTS.md"),
        fx.codex_home.join("plugins/p/plugin.json"),
        fx.codex_home.join("packages/x"),
        fx.codex_home.join(".tmp/x"),
    ] {
        assert!(!writable(&runtime), "{}", runtime.display());
        assert!(readable(&runtime), "{}", runtime.display());
    }
    let override_md = fx.codex_home.join("AGENTS.override.md");
    assert!(!writable(&override_md));
    assert!(!readable(&override_md));
}

/// #583: a protected command cannot create a missing Corbanu-home entry
/// that Corbanu reads later (exec-policy `rules/`, or any other name): the
/// home itself is read-only wherever the profile could write it, and no
/// grant lifts it. `worktrees` stays writable, also when the command works
/// in one, without becoming a root of its own there (whose default rules
/// would make a git worktree's git folder read-only). The profile is never
/// widened.
#[test]
fn issue_583_the_corbanu_home_takes_no_new_entries() {
    let fx = fixture();
    let project = fx.user_home.join("project");
    let worktree = fx.codex_home.join("worktrees/w");
    let created = [
        fx.codex_home.join("rules"),
        fx.codex_home.join("rules/x.rules"),
        fx.codex_home.join("hooks.json"),
        fx.codex_home.join("new.txt"),
    ];
    for (cwd, profile) in [
        (project.clone(), PermissionProfile::Disabled),
        (project.clone(), workspace(&fx.user_home)),
        (worktree.clone(), PermissionProfile::Disabled),
        (worktree.clone(), workspace(&worktree)),
    ] {
        let mut denials =
            ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)]);
        // An Aggressive grant lifts the other write rules, not this one.
        denials.lift_write_rules();
        let policy = denials
            .apply(&profile)
            .unwrap()
            .file_system_sandbox_policy();
        let writable = |path: &Path| policy.can_write_path_with_cwd(path, &cwd);
        for path in &created {
            assert!(!writable(path), "{}: {cwd:?} {profile:?}", path.display());
        }
        assert!(!writable(&fx.codex_home), "{cwd:?} {profile:?}");
        assert!(writable(&worktree.join("file")), "{cwd:?} {profile:?}");
        assert!(writable(&cwd.join("file")), "{cwd:?} {profile:?}");
        assert!(
            policy.can_read_path_with_cwd(&fx.codex_home.join("skills/s"), &cwd),
            "{cwd:?} {profile:?}"
        );
        if profile == PermissionProfile::Disabled {
            let roots: Vec<AbsolutePathBuf> = policy
                .get_writable_roots_with_cwd(&cwd)
                .into_iter()
                .map(|root| root.root)
                .collect();
            assert!(!roots.contains(&abs(&worktree)), "{roots:?}");
        }
    }
    // The profile is never widened: one that writes only the project keeps
    // the home's folders read-only.
    let project_only = PermissionProfile::from_runtime_permissions(
        &FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry::new(
                FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                FileSystemAccessMode::Read,
            ),
            FileSystemSandboxEntry::new(
                FileSystemPath::Path {
                    path: abs(&project),
                },
                FileSystemAccessMode::Write,
            ),
        ]),
        NetworkSandboxPolicy::Restricted,
    );
    let denials = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&project)]);
    let policy = denials
        .apply(&project_only)
        .unwrap()
        .file_system_sandbox_policy();
    assert!(!policy.can_write_path_with_cwd(&worktree.join("file"), &project));
    // Left as before: a person's approval of one command (#540), a command
    // working in the home itself, or in a folder of it outside `worktrees`.
    let other = fx.codex_home.join("other");
    std::fs::create_dir_all(&other).unwrap();
    for (cwd, runtime_only) in [
        (project, true),
        (fx.codex_home.clone(), false),
        (other, false),
    ] {
        let denials = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)]);
        let denials = if runtime_only {
            denials.runtime_only()
        } else {
            denials
        };
        let policy = denials
            .apply(&PermissionProfile::Disabled)
            .unwrap()
            .file_system_sandbox_policy();
        assert!(
            policy.can_write_path_with_cwd(&fx.codex_home.join("new.txt"), &cwd),
            "{cwd:?}"
        );
    }
}

/// #583 review: a `worktrees` link (to a runtime entry, or to the home
/// itself) opens nothing: only a real `worktrees` folder stays writable.
/// Where the profile could write the home, a missing `worktrees` is made.
#[test]
fn issue_583_a_worktrees_link_opens_nothing() {
    for target in ["tmp", "."] {
        let fx = fixture();
        std::fs::remove_dir_all(fx.codex_home.join("worktrees")).unwrap();
        std::os::unix::fs::symlink(target, fx.codex_home.join("worktrees")).unwrap();
        let cwd = fx.user_home.join("project");
        let mut denials =
            ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)]);
        denials.lift_write_rules();
        assert_eq!(denials.home_writable_paths().count(), 0, "{target}");
        let policy = denials
            .apply(&PermissionProfile::Disabled)
            .unwrap()
            .file_system_sandbox_policy();
        for path in ["tmp/x", "rules/x.rules", "worktrees/x"] {
            assert!(
                !policy.can_write_path_with_cwd(&fx.codex_home.join(path), &cwd),
                "{target}: {path}"
            );
        }
    }
    let fx = fixture();
    std::fs::remove_dir_all(fx.codex_home.join("worktrees")).unwrap();
    let cwd = abs(&fx.user_home.join("project"));
    ReadDenials::for_turn(
        &fx.codex_home,
        &cwd,
        std::slice::from_ref(&cwd),
        &PermissionProfile::read_only(),
    );
    assert!(!fx.codex_home.join("worktrees").exists());
    ReadDenials::for_turn(
        &fx.codex_home,
        &cwd,
        std::slice::from_ref(&cwd),
        &PermissionProfile::Disabled,
    );
    assert!(fx.codex_home.join("worktrees").is_dir());
}

/// Issue #562: on Linux the missing runtime folders are created, empty,
/// only where the profile could write the Corbanu home, so bubblewrap never
/// needs a placeholder file where Corbanu expects a folder. `AGENTS.md` is
/// never created.
#[test]
fn issue_562_missing_runtime_folders_are_created_only_where_writable() {
    let fx = fixture();
    let cwd = abs(&fx.user_home.join("project"));
    let folders = ["plugins", "packages", ".tmp"];
    ReadDenials::for_turn(
        &fx.codex_home,
        &cwd,
        std::slice::from_ref(&cwd),
        &PermissionProfile::read_only(),
    );
    for folder in folders {
        assert!(!fx.codex_home.join(folder).exists(), "{folder}");
    }
    ReadDenials::for_turn(
        &fx.codex_home,
        &cwd,
        std::slice::from_ref(&cwd),
        &PermissionProfile::Disabled,
    );
    for folder in folders {
        assert_eq!(
            fx.codex_home.join(folder).is_dir(),
            cfg!(target_os = "linux"),
            "{folder}"
        );
    }
    assert!(!fx.codex_home.join("AGENTS.md").exists());
    assert!(!fx.codex_home.join("AGENTS.override.md").exists());
}

/// PF-23-S02: files that run code later become read-only wherever the
/// command could write them; ordinary files stay writable.
#[test]
fn pf_23_s02_persistence_files_become_read_only() {
    let fx = fixture();
    let cwd = fx.user_home.join("project");
    std::fs::create_dir_all(cwd.join(".git/hooks")).unwrap();
    // An existing start-up file; the ones below do not exist.
    std::fs::write(fx.user_home.join(".bashrc"), "x").unwrap();
    let denials = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)]);
    let policy = denials
        .apply(&PermissionProfile::Disabled)
        .unwrap()
        .file_system_sandbox_policy();
    let writable = |path: &Path| policy.can_write_path_with_cwd(path, &cwd);
    let readable = |path: &Path| policy.can_read_path_with_cwd(path, &cwd);
    let missing_home_files = [
        fx.user_home.join(".zshrc"),
        fx.user_home.join(".config/fish/config.fish"),
        fx.user_home.join("Library/LaunchAgents/x.plist"),
        fx.user_home.join(".local/bin/corbanu"),
        fx.user_home.join(".claude/settings.json"),
    ];
    // Linux leaves missing home files to the command-text net: the sandbox
    // would put an empty placeholder in the real home for the command's run.
    for missing in &missing_home_files {
        assert_eq!(
            writable(missing),
            cfg!(target_os = "linux"),
            "{}",
            missing.display()
        );
    }
    for protected in [
        fx.user_home.join(".bashrc"),
        cwd.join(".git/hooks/pre-commit"),
        cwd.join(".git/config"),
        cwd.join(".codex/config.toml"),
        fx.codex_home.join("skills/x/SKILL.md"),
        fx.codex_home.join("shell_snapshots/s.sh"),
        fx.codex_home.join("tmp/arg0/apply_patch"),
    ] {
        assert!(!writable(&protected), "{}", protected.display());
        assert!(readable(&protected), "{}", protected.display());
    }
    for open in [
        cwd.join("src/main.rs"),
        cwd.join(".git/objects/ab"),
        fx.user_home.join("notes.txt"),
        fx.codex_home.join("worktrees/w/file"),
    ] {
        assert!(writable(&open), "{}", open.display());
    }
    // A credential stays unreadable inside a read-only folder.
    assert!(!readable(&fx.user_home.join(".claude/.credentials.json")));
}

/// PF-23-S02: the rules only narrow. A read-only entry never makes a path
/// readable that the profile did not let the command read, and every
/// denial of the profile itself stays.
#[test]
fn pf_23_s02_rules_never_widen_and_keep_existing_denials() {
    let fx = fixture();
    let cwd = fx.user_home.join("project");
    let secret = fx.user_home.join("private");
    let entry = |path: &Path, access| {
        FileSystemSandboxEntry::new(FileSystemPath::Path { path: abs(path) }, access)
    };
    let base = PermissionProfile::from_runtime_permissions(
        &FileSystemSandboxPolicy::restricted(vec![
            entry(&cwd, FileSystemAccessMode::Write),
            entry(&secret, FileSystemAccessMode::Deny),
        ]),
        NetworkSandboxPolicy::Restricted,
    );
    let denials = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)]);
    let policy = denials.apply(&base).unwrap().file_system_sandbox_policy();
    for unreadable in [fx.user_home.join(".zshrc"), fx.codex_home.join("skills")] {
        assert!(!policy.can_read_path_with_cwd(&unreadable, &cwd));
    }
    assert!(!policy.can_read_path_with_cwd(&secret, &cwd));
    assert!(policy.can_write_path_with_cwd(&cwd.join("a.txt"), &cwd));
    assert!(!policy.can_write_path_with_cwd(&cwd.join(".codex/config.toml"), &cwd));
}

/// PF-23-S02 / issue #239: under Aggressive and Moderate the rules apply
/// from the start of the session, before any untrusted content; Permissive
/// never gets them.
#[tokio::test]
async fn pf_23_s02_protected_levels_apply_the_rules_from_the_start() {
    let (session, turn) = crate::session::tests::make_session_and_context().await;
    let environment = turn
        .environments
        .primary()
        .expect("a primary environment")
        .clone();
    let auth = turn.config.codex_home.join("auth.json");
    #[allow(deprecated)]
    let cwd = turn.cwd.clone();
    let readable = |context: codex_file_system::FileSystemSandboxContext| {
        PermissionProfile::try_from(context.permissions)
            .unwrap()
            .file_system_sandbox_policy()
            .can_read_path_with_cwd(auth.as_path(), cwd.as_path())
    };
    // Issue #239: Moderate too, not only Aggressive.
    for (level, protected) in [
        (codex_security_policy::SecurityLevel::Permissive, false),
        (codex_security_policy::SecurityLevel::Moderate, true),
        (codex_security_policy::SecurityLevel::Aggressive, true),
    ] {
        let client = (*session.services.model_client())
            .clone()
            .with_ingress_level(level)
            .with_source_envelopes(true);
        session.services.replace_model_client(client);
        let context = turn.file_system_sandbox_context(None, &environment);
        let protected_context = protect_file_tool_context(&session, &turn, context.clone());
        assert_eq!(readable(protected_context), !protected, "{level:?}");
        assert_eq!(
            post_taint_read_policy(&session, &turn).is_some(),
            protected,
            "{level:?}"
        );
    }
}

/// PF-23-S02: worktrees keep hooks in the git folder their `.git` file
/// points to; a path below a regular file is left out (the Linux sandbox
/// cannot mount there); a symlinked dotfile is protected where it really is.
#[test]
fn pf_23_s02_worktree_hooks_and_symlinked_dotfiles_are_protected() {
    let fx = fixture();
    let main = fx.user_home.join("main");
    let common = main.join(".git");
    let own = common.join("worktrees/w");
    std::fs::create_dir_all(own.as_path()).unwrap();
    std::fs::create_dir_all(common.join("hooks")).unwrap();
    std::fs::write(own.join("commondir"), "../..\n").unwrap();
    let cwd = fx.user_home.join("w");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(cwd.join(".git"), format!("gitdir: {}\n", own.display())).unwrap();
    let dotfiles = fx.user_home.join("dotfiles");
    std::fs::create_dir_all(&dotfiles).unwrap();
    std::fs::write(dotfiles.join("zshrc"), "x").unwrap();
    std::os::unix::fs::symlink(dotfiles.join("zshrc"), fx.user_home.join(".zshrc")).unwrap();

    let denials = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)]);
    let read_only: Vec<_> = denials.read_only_paths().cloned().collect();
    let mut protected_paths = vec![
        cwd.join(".git"),
        common.join("hooks"),
        common.join("config"),
        own.join("config.worktree"),
        own.join("commondir"),
        dotfiles.join("zshrc"),
    ];
    if cfg!(target_os = "macos") {
        protected_paths.push(common.join("commondir"));
    }
    for protected in protected_paths {
        assert!(
            read_only.contains(&abs(&protected)),
            "{}",
            protected.display()
        );
    }
    // Nothing below the `.git` file.
    assert!(
        read_only
            .iter()
            .all(|path| path.as_path() == cwd.join(".git")
                || !path.as_path().starts_with(cwd.join(".git"))),
        "{read_only:?}"
    );
    let policy = denials
        .apply(&PermissionProfile::Disabled)
        .unwrap()
        .file_system_sandbox_policy();
    assert!(!policy.can_write_path_with_cwd(&common.join("hooks/pre-commit"), &cwd));
    assert!(!policy.can_write_path_with_cwd(&dotfiles.join("zshrc"), &cwd));
    assert!(policy.can_write_path_with_cwd(&cwd.join("src.rs"), &cwd));
}

/// PF-23-S02 review 2: a session in a subfolder still protects the
/// repository's hooks and config; a link to a missing target protects the
/// target's path.
#[test]
fn pf_23_s02_enclosing_repo_and_dangling_links_are_protected() {
    let fx = fixture();
    let repo = fx.user_home.join("repo");
    std::fs::create_dir_all(repo.join(".git/hooks")).unwrap();
    let cwd = repo.join("codex-rs");
    std::fs::create_dir_all(&cwd).unwrap();
    std::os::unix::fs::symlink(
        fx.user_home.join("dotfiles/zshrc"),
        fx.user_home.join(".zshrc"),
    )
    .unwrap();
    let denials = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)]);
    let read_only: Vec<_> = denials.read_only_paths().cloned().collect();
    let mut protected_paths = vec![repo.join(".git/hooks"), repo.join(".git/config")];
    if cfg!(target_os = "macos") {
        // Linux leaves out a missing commondir and a dangling home link.
        protected_paths.push(repo.join(".git/commondir"));
        protected_paths.push(fx.user_home.join("dotfiles/zshrc"));
    }
    for protected in protected_paths {
        assert!(
            read_only.contains(&abs(&protected)),
            "{}",
            protected.display()
        );
    }
}

/// PF-23-S02 follow-up: submodule git folders (nested ones and ones whose
/// path has a slash) and a `core.hooksPath` folder inside the workspace
/// (`.husky`) are read-only too; the rest of the workspace stays writable.
#[test]
fn pf_23_s02_submodules_and_hooks_path_are_protected() {
    let fx = fixture();
    let cwd = fx.user_home.join("project");
    let git = cwd.join(".git");
    for module in ["modules/a", "modules/a/modules/inner", "modules/libs/b"] {
        for folder in ["hooks", "objects"] {
            std::fs::create_dir_all(git.join(module).join(folder)).unwrap();
        }
        std::fs::write(git.join(module).join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git.join(module).join("config"), "").unwrap();
    }
    std::fs::write(
        git.join("config"),
        "[core]\n\tbare = false\n[core \"x\"]\n[CORE]\n\thooksPath = \".husky\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(cwd.join(".husky")).unwrap();

    let denials = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)]);
    let policy = denials
        .apply(&PermissionProfile::Disabled)
        .unwrap()
        .file_system_sandbox_policy();
    let writable = |path: &Path| policy.can_write_path_with_cwd(path, &cwd);
    for protected in [
        git.join("modules/a/hooks/pre-commit"),
        git.join("modules/a/config"),
        git.join("modules/a/modules/inner/hooks/post-checkout"),
        git.join("modules/libs/b/hooks/pre-push"),
        cwd.join(".husky/pre-commit"),
    ] {
        assert!(!writable(&protected), "{}", protected.display());
    }
    for open in [
        git.join("modules/a/objects/ab"),
        cwd.join("src/main.rs"),
        cwd.join(".huskyrc"),
    ] {
        assert!(writable(&open), "{}", open.display());
    }
}

/// Issue #239 regression: under a protected level (Moderate or Aggressive)
/// with a live policy and `source_envelopes` on, the read denials apply
/// *before* any untrusted content. A workspace-write sandbox otherwise lets
/// an agent read credential files from `$HOME` before the session is tainted.
#[tokio::test]
async fn issue_239_read_denials_apply_under_protected_level_before_taint() {
    use codex_security_policy::SecurityLevel;
    let (session, turn) = crate::session::tests::make_session_and_context().await;
    // Bind the effective security policy so the level is live (as the real
    // session does), not just an ingress-level hint.
    let control = session
        .services
        .agent_control
        .clone()
        .with_effective_security_policy(
            SecurityLevel::Moderate,
            session.thread_id,
            /*inherits_from_spawn_parent*/ false,
        )
        .expect("policy");
    let client = (*session.services.model_client())
        .clone()
        .with_ingress_policy(SecurityLevel::Moderate, control.effective_security_policy())
        .with_source_envelopes(true);
    session.services.replace_model_client(client);

    let environment = turn
        .environments
        .primary()
        .expect("a primary environment")
        .clone();
    let auth = turn.config.codex_home.join("auth.json");
    let readable = |context: codex_file_system::FileSystemSandboxContext| {
        let profile = PermissionProfile::try_from(context.permissions).unwrap();
        #[allow(deprecated)]
        let cwd = turn.cwd.clone();
        profile
            .file_system_sandbox_policy()
            .can_read_path_with_cwd(auth.as_path(), cwd.as_path())
    };
    let context = || turn.file_system_sandbox_context(None, &environment);

    // No taint has occurred yet, but the level is Moderate: denials apply.
    assert_eq!(
        session
            .services
            .model_client()
            .post_taint_state()
            .expect("source envelopes on")
            .taint_generation,
        0
    );
    let before_taint = protect_file_tool_context(&session, &turn, context());
    assert!(
        readable(context()),
        "baseline workspace-write reads the credential file"
    );
    assert!(
        !readable(before_taint),
        "issue #239: read denials must apply under a protected level before taint"
    );

    // The host upload read policy is also active before taint under a level.
    let policy = post_taint_read_policy(&session, &turn).expect("protected level active");
    #[allow(deprecated)]
    let cwd = turn.cwd.clone();
    assert!(
        !readable_under(&policy, &auth, cwd.as_path()),
        "issue #239: upload read policy must deny credentials before taint"
    );
}

/// #442: under Aggressive a grant lifts the write protections only. Reads of
/// credential files and the Corbanu home stay denied, not even once granted.
#[test]
fn issue_442_a_grant_keeps_every_read_denial() {
    let fx = fixture();
    let cwd = fx.user_home.join("project");
    std::fs::create_dir_all(cwd.join(".git/hooks")).unwrap();
    let mut denials = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)]);
    denials.lift_write_rules();
    let policy = denials
        .apply(&PermissionProfile::Disabled)
        .unwrap()
        .file_system_sandbox_policy();
    for denied in [
        fx.user_home.join(".ssh/id_ed25519"),
        fx.user_home.join(".aws/credentials"),
        fx.codex_home.join("auth.json"),
        fx.codex_home.join("config.toml"),
        fx.codex_home.join("secrets/local.age"),
    ] {
        assert!(
            !policy.can_read_path_with_cwd(&denied, &cwd),
            "{}",
            denied.display()
        );
    }
    assert!(policy.can_write_path_with_cwd(&cwd.join(".git/hooks/pre-commit"), &cwd));
    // The Corbanu home's runtime entries (sandbox launchers in `tmp`) stay
    // read-only even with a grant, wherever the command could write.
    let full = denials
        .apply(&PermissionProfile::Disabled)
        .unwrap()
        .file_system_sandbox_policy();
    for runtime in [
        fx.codex_home.join("tmp/arg0/codex-linux-sandbox"),
        fx.codex_home.join("skills/x/SKILL.md"),
        fx.codex_home.join("shell_snapshots/s.sh"),
    ] {
        assert!(
            !full.can_write_path_with_cwd(&runtime, &cwd),
            "{}",
            runtime.display()
        );
    }
}

/// #512: the upload check resolves links on both sides. A Corbanu home
/// reached through a link (`/tmp` and `/var` on macOS, a linked home) spells
/// its denials through the link; neither a link to a denied file nor the
/// file's real name may pass. A more specific allow inside a denied folder
/// still applies under either spelling, and nothing outside is narrowed.
#[test]
fn issue_512_upload_reads_resolve_links_on_both_sides() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let real_home = base.join("real-home");
    std::fs::create_dir_all(real_home.join("vault")).unwrap();
    std::fs::write(real_home.join("auth.json"), "x").unwrap();
    std::fs::write(real_home.join("vault/key"), "x").unwrap();
    std::fs::write(real_home.join("vault/open.txt"), "x").unwrap();
    let linked_home = base.join("linked-home");
    std::os::unix::fs::symlink(&real_home, &linked_home).unwrap();
    let outside = base.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let to_auth = outside.join("innocent.txt");
    std::os::unix::fs::symlink(linked_home.join("auth.json"), &to_auth).unwrap();
    let plain = outside.join("plain.txt");
    std::fs::write(&plain, "x").unwrap();

    let entry = |path: &Path, access| {
        FileSystemSandboxEntry::new(FileSystemPath::Path { path: abs(path) }, access)
    };
    let policy = FileSystemSandboxPolicy::restricted(vec![
        FileSystemSandboxEntry::new(
            FileSystemPath::Special {
                value: codex_protocol::permissions::FileSystemSpecialPath::Root,
            },
            FileSystemAccessMode::Read,
        ),
        entry(&linked_home.join("auth.json"), FileSystemAccessMode::Deny),
        entry(&linked_home.join("vault"), FileSystemAccessMode::Deny),
        entry(
            &linked_home.join("vault/open.txt"),
            FileSystemAccessMode::Read,
        ),
    ]);
    let cwd = outside.as_path();
    for denied in [
        linked_home.join("auth.json"),
        real_home.join("auth.json"),
        to_auth,
        linked_home.join("vault/key"),
        real_home.join("vault/key"),
    ] {
        assert!(
            !readable_under(&policy, &denied, cwd),
            "{}",
            denied.display()
        );
    }
    for open in [
        plain,
        linked_home.join("vault/open.txt"),
        real_home.join("vault/open.txt"),
    ] {
        assert!(readable_under(&policy, &open, cwd), "{}", open.display());
    }
}

/// #506: with the home's listing denied, the sandbox gets a listing denial
/// for the Corbanu home and nothing else changes: its runtime entries stay
/// readable, its stores stay denied. Without it (Moderate), no listing
/// denial; a command working in the home itself keeps listing it.
#[test]
fn issue_506_home_listing_denial_is_added_only_when_asked() {
    let fx = fixture();
    let cwd = fx.user_home.join("project");
    let home = abs(&fx.codex_home);
    let unlistable = |denials: &ReadDenials| {
        denials
            .apply(&workspace(&cwd))
            .unwrap()
            .file_system_sandbox_policy()
            .get_unlistable_directories()
    };
    let mut denials = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)]);
    assert!(unlistable(&denials).is_empty());
    denials.deny_home_listing();
    assert_eq!(unlistable(&denials), vec![home.clone()]);
    let policy = denials
        .apply(&workspace(&cwd))
        .unwrap()
        .file_system_sandbox_policy();
    assert!(policy.can_read_path_with_cwd(&fx.codex_home.join("skills"), &cwd));
    assert!(!policy.can_read_path_with_cwd(&fx.codex_home.join("auth.json"), &cwd));

    // Full access gets a sandbox that only denies reads and the listing.
    let full = denials.apply(&PermissionProfile::Disabled).unwrap();
    let full = full.file_system_sandbox_policy();
    assert!(full.can_write_path_with_cwd(&cwd.join("file.txt"), &cwd));
    assert_eq!(full.get_unlistable_directories(), vec![home.clone()]);

    let mut in_home = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[home]);
    in_home.deny_home_listing();
    assert_eq!(in_home.unlistable_home(), None);
}

/// #506: under Aggressive, in-process file tools (which list folders inside
/// the sandbox) cannot list the Corbanu home either; under Moderate they can.
#[tokio::test]
async fn issue_506_file_tools_cannot_list_the_home_under_aggressive() {
    use codex_security_policy::SecurityLevel;
    for (level, denied) in [
        (SecurityLevel::Aggressive, true),
        (SecurityLevel::Moderate, false),
    ] {
        let (session, turn) = crate::session::tests::make_session_and_context().await;
        let client = (*session.services.model_client())
            .clone()
            .with_ingress_level(level)
            .with_source_envelopes(true);
        session.services.replace_model_client(client);
        let environment = turn
            .environments
            .primary()
            .expect("a primary environment")
            .clone();
        let context = turn.file_system_sandbox_context(None, &environment);
        let protected = protect_file_tool_context(&session, &turn, context);
        let profile = PermissionProfile::try_from(protected.permissions).unwrap();
        let unlistable = profile
            .file_system_sandbox_policy()
            .get_unlistable_directories();
        // The home as the sandbox sees it (its real location).
        assert_eq!(
            unlistable
                .iter()
                .any(|path| path.as_path().file_name()
                    == turn.config.codex_home.as_path().file_name()),
            denied,
            "{level:?}: {unlistable:?}"
        );
        assert!(unlistable.len() <= 1, "{unlistable:?}");
    }
}

/// #540: where a person's approval lifts Moderate's rules for one command,
/// only the runtime entries' rule stays: they remain read-only, also under
/// full access (which then gets a sandbox for it), while credential reads
/// and persistence writes are lifted.
#[test]
fn issue_540_runtime_entries_stay_read_only_when_the_rules_are_lifted() {
    let fx = fixture();
    let cwd = fx.user_home.join("project");
    std::fs::write(fx.user_home.join(".bashrc"), "x").unwrap();
    for profile in [workspace(&fx.user_home), PermissionProfile::Disabled] {
        let all = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)]);
        assert!(all.read_only_paths().count() > all.runtime_read_only_paths().count());
        let denials = all.runtime_only();
        assert_eq!(denials.paths().count(), 0);
        assert_eq!(
            denials.read_only_paths().count(),
            denials.runtime_read_only_paths().count()
        );
        assert!(denials.matcher().is_none());
        let lifted = denials.apply(&profile).expect("not an external sandbox");
        let policy = lifted.file_system_sandbox_policy();
        assert_eq!(
            policy.kind,
            FileSystemSandboxKind::Restricted,
            "{profile:?}"
        );
        let writable = |path: &Path| policy.can_write_path_with_cwd(path, &cwd);
        for runtime in ["tmp", "tmp/arg0/launcher", "shell_snapshots", "skills"] {
            assert!(
                !writable(&fx.codex_home.join(runtime)),
                "{runtime}: {profile:?}"
            );
        }
        // Persistence files are lifted (the profile's own rules aside).
        assert!(writable(&fx.user_home.join(".bashrc")), "{profile:?}");
        assert!(writable(&cwd.join("notes.txt")), "{profile:?}");
        assert!(
            policy.can_read_path_with_cwd(&fx.user_home.join(".ssh/id_ed25519"), &cwd),
            "{profile:?}"
        );
        assert!(
            policy.can_read_path_with_cwd(&fx.codex_home.join("auth.json"), &cwd),
            "{profile:?}"
        );
    }
    // Full access keeps its network.
    let lifted = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)])
        .runtime_only()
        .apply(&PermissionProfile::Disabled)
        .unwrap();
    assert_eq!(
        lifted.network_sandbox_policy(),
        NetworkSandboxPolicy::Enabled
    );
}

/// A user home reached through a link (`user -> real-user`) holding a
/// dummy SSH key, with a workspace whose `.agents` links to that key.
struct LinkedHome {
    _dir: tempfile::TempDir,
    real_home: std::path::PathBuf,
    linked_home: std::path::PathBuf,
    codex_home: std::path::PathBuf,
    cwd: std::path::PathBuf,
}

fn linked_home_with_agents_link_to_key() -> LinkedHome {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let real_home = base.join("real-user");
    std::fs::create_dir_all(real_home.join(".ssh")).unwrap();
    std::fs::write(real_home.join(".ssh/id_ed25519"), "DUMMY_SSH_KEY_518").unwrap();
    std::fs::create_dir_all(real_home.join("project")).unwrap();
    std::fs::write(real_home.join("project/README.md"), "readme").unwrap();
    std::os::unix::fs::symlink("../.ssh/id_ed25519", real_home.join("project/.agents")).unwrap();
    let linked_home = base.join("user");
    std::os::unix::fs::symlink(&real_home, &linked_home).unwrap();
    let codex_home = linked_home.join(".corbanu");
    std::fs::create_dir_all(codex_home.join("tmp")).unwrap();
    let cwd = linked_home.join("project");
    LinkedHome {
        _dir: dir,
        real_home,
        linked_home,
        codex_home,
        cwd,
    }
}

/// #518: a read-only entry never reopens a denied file. With the home
/// reached through a link, `.agents -> ../.ssh/id_ed25519` became a `Read`
/// entry at the key's real path, more specific than the `.ssh` denial
/// spelled through the link, so a full-write profile could read the key.
#[test]
fn issue_518_read_only_entries_never_reopen_a_denial_through_a_linked_home() {
    let fx = linked_home_with_agents_link_to_key();
    let cwd = fx.cwd.as_path();
    let denials = ReadDenials::collect(&fx.codex_home, Some(&fx.linked_home), None, &[abs(cwd)]);
    for profile in [PermissionProfile::Disabled, workspace(cwd)] {
        let policy = denials
            .apply(&profile)
            .unwrap()
            .file_system_sandbox_policy();
        let real_key = fx.real_home.join(".ssh/id_ed25519");
        // No entry at or above the key's real path grants a read the
        // denial does not: the sandboxes see the more specific entry first.
        assert!(
            !policy.entries.iter().any(|entry| entry.access.can_read()
                && matches!(&entry.path, FileSystemPath::Path { path }
                    if real_key.starts_with(path.as_path()) && path.as_path() != Path::new("/"))),
            "{profile:?}: {:?}",
            policy.entries
        );
        for key in [
            real_key.clone(),
            fx.linked_home.join(".ssh/id_ed25519"),
            cwd.join(".agents"),
            fx.real_home.join("project/.agents"),
        ] {
            assert!(
                !readable_under(&policy, &key, cwd),
                "{profile:?}: {}",
                key.display()
            );
        }
        assert!(
            !policy.can_write_path_with_cwd(&real_key, cwd),
            "{profile:?}"
        );
        assert!(readable_under(&policy, &cwd.join("README.md"), cwd));
    }
    // Nothing else is narrowed: the runtime entries stay readable in both
    // spellings, and read-only at their real location (where every
    // platform keeps the entry; macOS also keeps the link spelling).
    let policy = denials
        .apply(&PermissionProfile::Disabled)
        .unwrap()
        .file_system_sandbox_policy();
    let real_runtime = fx.real_home.join(".corbanu/tmp");
    for runtime in [fx.codex_home.join("tmp"), real_runtime.clone()] {
        assert!(
            readable_under(&policy, &runtime.join("arg0"), cwd),
            "{runtime:?}"
        );
    }
    assert!(!policy.can_write_path_with_cwd(&real_runtime.join("arg0"), cwd));
}

/// #518 review: a workspace link to a key that does not exist yet is
/// checked where it would lead, so it never becomes readable when created.
#[test]
fn issue_518_a_dangling_link_to_a_denied_folder_stays_denied() {
    let fx = linked_home_with_agents_link_to_key();
    let cwd = fx.cwd.as_path();
    std::fs::remove_file(cwd.join(".agents")).unwrap();
    std::os::unix::fs::symlink("../.ssh/id_new", cwd.join(".agents")).unwrap();
    let denials = ReadDenials::collect(&fx.codex_home, Some(&fx.linked_home), None, &[abs(cwd)]);
    let policy = denials
        .apply(&PermissionProfile::Disabled)
        .unwrap()
        .file_system_sandbox_policy();
    assert!(!readable_under(&policy, &cwd.join(".agents"), cwd));
    let new_key = fx.real_home.join(".ssh/id_new");
    assert!(!policy.entries.iter().any(|entry| entry.access.can_read()
        && matches!(&entry.path, FileSystemPath::Path { path } if path.as_path() == new_key)));
    std::fs::write(&new_key, "DUMMY").unwrap();
    assert!(!readable_under(&policy, &cwd.join(".agents"), cwd));
    assert!(!readable_under(&policy, &new_key, cwd));
}

/// #518 review: another Corbanu home name linking to the active home is
/// the active home: its runtime entries stay readable, its stores denied.
#[test]
fn issue_518_a_home_alias_link_keeps_runtime_entries_readable() {
    let fx = fixture();
    let alias = fx.user_home.join(".codex");
    std::os::unix::fs::symlink(&fx.codex_home, &alias).unwrap();
    let cwd = fx.user_home.join("project");
    let denials = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)]);
    assert!(!denials.paths().any(|path| path.as_path() == alias));
    let policy = denials
        .apply(&PermissionProfile::Disabled)
        .unwrap()
        .file_system_sandbox_policy();
    for runtime in [fx.codex_home.join("tmp/arg0"), alias.join("tmp/arg0")] {
        assert!(readable_under(&policy, &runtime, &cwd), "{runtime:?}");
    }
    for store in [fx.codex_home.join("auth.json"), alias.join("auth.json")] {
        assert!(!readable_under(&policy, &store, &cwd), "{store:?}");
    }
}

/// #518 with the real macOS sandbox: a full-write command could `cat` the
/// key through `.agents` and under its real name.
#[cfg(target_os = "macos")]
#[test]
fn issue_518_sandboxed_commands_cannot_read_the_key_through_a_linked_home() {
    use codex_sandboxing::seatbelt::CreateSeatbeltCommandArgsParams;
    use codex_sandboxing::seatbelt::MACOS_PATH_TO_SEATBELT_EXECUTABLE;
    use codex_sandboxing::seatbelt::create_seatbelt_command_args;

    let fx = linked_home_with_agents_link_to_key();
    let cwd = fx.cwd.as_path();
    let denials = ReadDenials::collect(&fx.codex_home, Some(&fx.linked_home), None, &[abs(cwd)]);
    let policy = denials
        .apply(&PermissionProfile::Disabled)
        .unwrap()
        .file_system_sandbox_policy();
    let script = format!(
        "cat .agents; echo agents=$?; cat '{real}'; echo real=$?; cat '{link}'; echo link=$?; \
         cat README.md; echo readme=$?",
        real = fx.real_home.join(".ssh/id_ed25519").display(),
        link = fx.linked_home.join(".ssh/id_ed25519").display(),
    );
    let args = create_seatbelt_command_args(CreateSeatbeltCommandArgsParams {
        command: vec!["/bin/sh".to_string(), "-c".to_string(), script],
        file_system_sandbox_policy: &policy,
        network_sandbox_policy: NetworkSandboxPolicy::Restricted,
        sandbox_policy_cwd: cwd,
        enforce_managed_network: false,
        managed_network: None,
        environment_id: None,
        network: None,
        extra_allow_unix_sockets: &[],
    })
    .unwrap();
    let output = std::process::Command::new(MACOS_PATH_TO_SEATBELT_EXECUTABLE)
        .args(&args)
        .current_dir(cwd)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("DUMMY_SSH_KEY_518"), "{stdout}");
    for line in ["agents=1", "real=1", "link=1", "readme=0"] {
        assert!(stdout.contains(line), "{line} not in {stdout}");
    }
}

/// #518 review: a dangling link whose target climbs through another link
/// (`d/../.ssh/id_new` with `d -> ../Documents`) is checked where the
/// system would resolve it, and a chain too long to follow is not readable.
#[test]
fn issue_518_dangling_links_resolve_through_the_folders_they_name() {
    let fx = linked_home_with_agents_link_to_key();
    let cwd = fx.cwd.as_path();
    std::fs::create_dir_all(fx.real_home.join("Documents")).unwrap();
    std::os::unix::fs::symlink("../Documents", cwd.join("d")).unwrap();
    std::fs::remove_file(cwd.join(".agents")).unwrap();
    std::os::unix::fs::symlink("d/../.ssh/id_new", cwd.join(".agents")).unwrap();
    for hop in 0..9 {
        let next = if hop == 8 {
            "missing".to_string()
        } else {
            format!("chain{}", hop + 1)
        };
        std::os::unix::fs::symlink(&next, cwd.join(format!("chain{hop}"))).unwrap();
    }
    let denials = ReadDenials::collect(&fx.codex_home, Some(&fx.linked_home), None, &[abs(cwd)]);
    let policy = denials
        .apply(&PermissionProfile::Disabled)
        .unwrap()
        .file_system_sandbox_policy();
    assert!(!readable_under(&policy, &cwd.join(".agents"), cwd));
    assert!(!readable_under(&policy, &cwd.join("chain0"), cwd));
    assert!(readable_under(&policy, &cwd.join("not-yet.txt"), cwd));
}

/// #518 review: a home name whose real folder only holds the active home
/// (`~/.corbanu -> ~/.codex/corbanu`) stays denied whole: its own stores
/// are not the active home's.
#[test]
fn issue_518_a_home_name_above_the_active_home_stays_denied() {
    let dir = tempfile::tempdir().unwrap();
    let user_home = dir.path().canonicalize().unwrap();
    let legacy = user_home.join(".codex");
    let codex_home = legacy.join("corbanu");
    std::fs::create_dir_all(codex_home.join("tmp")).unwrap();
    std::fs::write(legacy.join("auth.json"), "DUMMY").unwrap();
    std::os::unix::fs::symlink(&codex_home, user_home.join(".corbanu")).unwrap();
    let active = user_home.join(".corbanu");
    let cwd = user_home.join("project");
    std::fs::create_dir_all(&cwd).unwrap();
    let denials = ReadDenials::collect(&active, Some(&user_home), None, &[abs(&cwd)]);
    let policy = denials
        .apply(&workspace(&cwd))
        .unwrap()
        .file_system_sandbox_policy();
    assert!(!readable_under(&policy, &legacy.join("auth.json"), &cwd));
}

/// #442 (Travis, 2026-10-11): a grant never lifts the project's `.codex` and
/// `.agents`, existing or not; it still lifts git hooks and config.
#[test]
fn issue_442_a_grant_keeps_codex_and_agents_read_only() {
    let fx = fixture();
    let cwd = fx.user_home.join("project");
    std::fs::create_dir_all(cwd.join(".git/hooks")).unwrap();
    std::fs::create_dir_all(cwd.join(".codex")).unwrap();
    let mut denials = ReadDenials::collect(&fx.codex_home, Some(&fx.user_home), None, &[abs(&cwd)]);
    denials.lift_write_rules();
    let policy = denials
        .apply(&PermissionProfile::Disabled)
        .unwrap()
        .file_system_sandbox_policy();
    for kept in [
        ".codex/config.toml",
        ".codex",
        ".agents/skills/x/SKILL.md",
        ".agents",
    ] {
        assert!(
            !policy.can_write_path_with_cwd(&cwd.join(kept), &cwd),
            "{kept}"
        );
    }
    for lifted in [".git/hooks/pre-commit", ".git/config"] {
        assert!(
            policy.can_write_path_with_cwd(&cwd.join(lifted), &cwd),
            "{lifted}"
        );
    }
}
