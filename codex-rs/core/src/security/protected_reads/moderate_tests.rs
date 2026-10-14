use super::*;
use codex_protocol::models::PermissionProfile;
use codex_protocol::permissions::FileSystemAccessMode;
use codex_protocol::permissions::FileSystemPath;
use codex_protocol::permissions::FileSystemSandboxEntry;
use codex_protocol::permissions::NetworkSandboxPolicy;
use pretty_assertions::assert_eq;

/// A Corbanu home holding `auth.json`, a project beside it, and the inputs
/// a Moderate session captures for them.
struct Fixture {
    _dirs: (tempfile::TempDir, tempfile::TempDir),
    home: PathBuf,
    project: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().expect("home");
        let project = tempfile::tempdir().expect("project");
        let (home_path, project_path) = (
            home.path().canonicalize().expect("home path"),
            project.path().canonicalize().expect("project path"),
        );
        std::fs::write(home_path.join("auth.json"), "{}").expect("auth.json");
        std::fs::create_dir(project_path.join(".codex")).expect(".codex");
        Self {
            _dirs: (home, project),
            home: home_path,
            project: project_path,
        }
    }

    /// The workspace-write profile without the temporary folders, so a
    /// write beside the project is the person's own sandbox's denial.
    fn profile() -> PermissionProfile {
        PermissionProfile::workspace_write_with(
            &[],
            NetworkSandboxPolicy::Restricted,
            /*exclude_tmpdir_env_var*/ true,
            /*exclude_slash_tmp*/ true,
        )
    }

    fn inputs(&self, profile: PermissionProfile) -> ReadPolicyInputs {
        ReadPolicyInputs {
            codex_home: self.home.clone(),
            sqlite_home: self.home.clone(),
            turn_cwd: AbsolutePathBuf::from_absolute_path(&self.project).expect("cwd"),
            workspace_roots: Vec::new(),
            profile,
            user_home: dirs::home_dir(),
        }
    }

    fn contract(&self) -> LaunchContract {
        LaunchContract::capture(
            &AbsolutePathBuf::from_absolute_path(&self.home).expect("home"),
            std::iter::empty(),
            /*hardened*/ false,
        )
    }

    fn explain(
        &self,
        profile: PermissionProfile,
        contract: Option<&LaunchContract>,
        script: &str,
        line: &str,
    ) -> Option<String> {
        explain(
            &self.inputs(profile),
            contract,
            &[
                "/bin/bash".to_string(),
                "-c".to_string(),
                script.to_string(),
            ],
            &self.project,
            &[line.to_string()],
        )
    }
}

/// #748 (c6, c3): a read of Corbanu's sign-in (or vault, or config) that
/// Moderate's rules denied names Moderate; secretless launch keeps it at
/// every level, so then switching to Permissive is not offered.
#[test]
fn issue_748_moderate_read_denial_names_the_level() {
    let fixture = Fixture::new();
    let auth = format!("{}/auth.json", fixture.home.display());
    let script = format!("cat '{auth}'");
    let line = format!("cat: {auth}: Operation not permitted");

    let explained = fixture.explain(Fixture::profile(), None, &script, &line);
    assert_eq!(
        explained,
        Some(read_explanation(&auth, /*every_level*/ false))
    );
    let explained = explained.expect("explained");
    assert!(
        explained.starts_with("Security level Moderate blocked access to"),
        "{explained}"
    );
    assert!(explained.contains("Permissive in /security"), "{explained}");

    let contract = fixture.contract();
    let explained = fixture.explain(Fixture::profile(), Some(&contract), &script, &line);
    assert_eq!(
        explained,
        Some(read_explanation(&auth, /*every_level*/ true))
    );
    let explained = explained.expect("explained");
    assert!(explained.contains("at every security level"), "{explained}");
    assert!(!explained.contains("Permissive"), "{explained}");

    // A failure that names no denied path is not the level's.
    assert_eq!(
        fixture.explain(
            Fixture::profile(),
            None,
            &script,
            "cat: /etc/other.txt: Operation not permitted"
        ),
        None
    );
}

/// #748: with the person's own full access, a write to the project's
/// `.codex` that Moderate's rules denied names Moderate. Under secretless
/// launch no level runs the command outside a sandbox, so then switching to
/// Permissive is not offered.
#[test]
fn issue_748_moderate_write_denial_names_the_level() {
    let fixture = Fixture::new();
    let script = "printf x > .codex/p.toml";
    let line = "bash: .codex/p.toml: Operation not permitted";
    assert_eq!(
        fixture.explain(PermissionProfile::Disabled, None, script, line),
        Some(write_explanation(
            ".codex/p.toml",
            /*every_level*/ false
        ))
    );
    let contract = fixture.contract();
    assert_eq!(
        fixture.explain(PermissionProfile::Disabled, Some(&contract), script, line),
        Some(write_explanation(
            ".codex/p.toml",
            /*every_level*/ true
        ))
    );
    // Workspace-write keeps `.codex` read-only itself: not Moderate's.
    assert_eq!(
        fixture.explain(Fixture::profile(), None, script, line),
        None
    );
}

/// #748: what Moderate's rules did not add is not attributed to the level:
/// a write outside the project (the person's own sandbox), and a read the
/// person's own profile already denies.
#[test]
fn issue_748_denials_moderate_did_not_add_are_not_attributed() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.explain(
            Fixture::profile(),
            None,
            "printf x > ../outside-notes.txt",
            "bash: ../outside-notes.txt: Operation not permitted"
        ),
        None
    );

    let auth = fixture.home.join("auth.json");
    let (mut file_system, network) = Fixture::profile().to_runtime_permissions();
    file_system.entries.push(FileSystemSandboxEntry::new(
        FileSystemPath::Path {
            path: AbsolutePathBuf::from_absolute_path(&auth).expect("auth"),
        },
        FileSystemAccessMode::Deny,
    ));
    let own_denial = PermissionProfile::from_runtime_permissions(&file_system, network);
    let auth = auth.display().to_string();
    assert_eq!(
        fixture.explain(
            own_denial,
            None,
            &format!("cat '{auth}'"),
            &format!("cat: {auth}: Operation not permitted")
        ),
        None
    );
}
