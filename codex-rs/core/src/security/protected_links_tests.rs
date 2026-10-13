use super::*;
use codex_protocol::models::PermissionProfile;
use codex_protocol::permissions::NetworkSandboxPolicy;
use pretty_assertions::assert_eq;

fn shell(script: &str) -> Vec<String> {
    vec!["/bin/zsh".into(), "-lc".into(), script.into()]
}

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_string()).collect()
}

fn targets(command: &[String]) -> Vec<String> {
    link_targets(command)
        .into_iter()
        .map(|candidate| candidate.target)
        .collect()
}

#[test]
fn issue_623_finds_what_ln_cp_and_windows_links_point_to() {
    assert_eq!(targets(&shell("ln -s .agents ag")), vec![".agents"]);
    assert_eq!(
        targets(&shell("ln -s .agents ag && echo x >> ag/notes.md")),
        vec![".agents"]
    );
    assert_eq!(targets(&shell("ln -sfn '/p/.codex' x")), vec!["/p/.codex"]);
    assert_eq!(
        targets(&shell("ln --symbolic -- .agents ag")),
        vec![".agents"]
    );
    assert_eq!(targets(&shell("ln -s .agents")), vec![".agents"]);
    assert_eq!(
        targets(&shell("ln -s -t links .codex .agents")),
        vec![".codex", ".agents"]
    );
    assert_eq!(
        targets(&shell("ln .agents/notes.md h")),
        vec![".agents/notes.md"]
    );
    assert_eq!(
        targets(&shell("cd sub; ln -s ../.agents a")),
        vec!["../.agents"]
    );
    assert_eq!(
        targets(&shell("cp -s .agents/notes.md x")),
        vec![".agents/notes.md"]
    );
    assert_eq!(targets(&shell("cp -r .agents x")), Vec::<String>::new());
    assert_eq!(
        targets(&shell("bash -c 'sudo ln -s .codex c'")),
        vec![".codex"]
    );
    assert_eq!(
        targets(&argv(&["cmd.exe", "/c", "mklink", "/D", "ag", ".agents"])),
        vec![".agents"]
    );
    assert_eq!(
        targets(&argv(&[
            "powershell.exe",
            "-Command",
            "New-Item -ItemType SymbolicLink -Path ag -Target .agents"
        ])),
        vec![".agents"]
    );
    assert_eq!(
        targets(&shell("ls -la ag; cat notes.txt")),
        Vec::<String>::new()
    );
    // The link's own name is not a target.
    let found = link_targets(&shell("ln -s .agents sub/ag"));
    assert_eq!(
        found[0].link_folders,
        vec!["sub".to_string(), "sub/ag".to_string()]
    );
}

#[test]
fn issue_623_finds_links_made_by_interpreters() {
    assert_eq!(
        targets(&shell(
            "python3 -c \"import os; os.symlink('.agents', 'ag')\""
        )),
        vec![".agents", "ag"]
    );
    assert_eq!(
        targets(&argv(&[
            "python3.12",
            "-c",
            "import pathlib; pathlib.Path('ag').symlink_to('/p/.codex')"
        ])),
        vec!["ag", "/p/.codex"]
    );
    assert_eq!(
        targets(&argv(&[
            "node",
            "-e",
            "require('fs').symlinkSync(\".agents\", 'x')"
        ])),
        vec!["fs", "x", ".agents"]
    );
    assert_eq!(
        targets(&shell("perl -e 'symlink(\".agents\", \"x\")'")),
        vec![".agents", "x"]
    );
    assert!(
        targets(&shell(
            "python3 - <<'EOF'\nimport os\nos.symlink('.agents', 'ag')\nEOF"
        ))
        .contains(&".agents".to_string())
    );
    assert!(
        targets(&shell(
            "python3 -c \"import os\nos.symlink('.agents', 'ag')\""
        ))
        .contains(&".agents".to_string())
    );
    assert_eq!(targets(&shell("x\nln -s .agents ag")), vec![".agents"]);
    // Code that makes no link names nothing.
    assert_eq!(
        targets(&shell(
            "python3 -c \"print(open('.agents/notes.md').read())\""
        )),
        Vec::<String>::new()
    );
}

struct Project {
    _home: tempfile::TempDir,
    _root: tempfile::TempDir,
    root: PathBuf,
    inputs: Inputs,
}

fn project() -> Project {
    let home = tempfile::TempDir::new().expect("home");
    let root_dir = tempfile::TempDir::new().expect("project");
    let root = root_dir.path().canonicalize().expect("real project");
    std::fs::create_dir_all(root.join(".agents")).expect(".agents");
    std::fs::create_dir_all(root.join("sub")).expect("sub");
    let turn_cwd = AbsolutePathBuf::from_absolute_path(&root).expect("absolute");
    let inputs = Inputs {
        codex_home: home.path().canonicalize().expect("real home"),
        turn_cwd: turn_cwd.clone(),
        workspace_roots: vec![turn_cwd],
        profile: PermissionProfile::workspace_write_with(
            &[],
            NetworkSandboxPolicy::Restricted,
            /*exclude_tmpdir_env_var*/ true,
            /*exclude_slash_tmp*/ true,
        ),
        user_home: None,
    };
    Project {
        _home: home,
        _root: root_dir,
        root,
        inputs,
    }
}

fn refused(project: &Project, command: &[String], cwd: &Path) -> Option<String> {
    project.inputs.protected_target(&link_targets(command), cwd)
}

#[test]
fn issue_623_refuses_relative_and_absolute_links_into_agent_settings() {
    let project = project();
    let root = project.root.as_path();
    let absolute = format!("ln -s {}/.codex c", root.display());
    assert_eq!(
        refused(&project, &shell("ln -s .agents ag"), root),
        Some(".agents".to_string())
    );
    assert_eq!(
        refused(&project, &shell(&absolute), root),
        Some(format!("{}/.codex", root.display()))
    );
    // Relative to the link's folder, and from a folder below the project.
    assert_eq!(
        refused(&project, &shell("ln -s ../.agents sub/ag"), root),
        Some("../.agents".to_string())
    );
    assert_eq!(
        refused(&project, &shell("ln -s ../.agents ag"), &root.join("sub")),
        Some("../.agents".to_string())
    );
    assert_eq!(
        refused(
            &project,
            &shell("python3 -c \"import os; os.symlink('.agents', 'ag')\""),
            root
        ),
        Some(".agents".to_string())
    );
    // A link to the Corbanu home is refused too.
    let home = format!("ln -s {} h", project.inputs.codex_home.display());
    assert!(refused(&project, &shell(&home), root).is_some());
}

#[test]
fn issue_623_other_links_are_not_refused() {
    let project = project();
    let root = project.root.as_path();
    for script in [
        "ln -s src/notes.py n",
        "ln -s sub/.agents x",
        "ln -s .agents-old x",
        "ln -s . self",
        "python3 -c \"import os; os.symlink('notes.txt', 'n')\"",
        "cat .agents/notes.md",
    ] {
        assert_eq!(refused(&project, &shell(script), root), None, "{script}");
    }
}
