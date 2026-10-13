use super::*;
#[cfg(unix)]
use codex_protocol::models::PermissionProfile;
#[cfg(unix)]
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
        vec!["fs", ".agents", "x"]
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

/// Review r1: commands behind shell keywords, grouping and wrappers, `-t`
/// in an option cluster, heredocs behind a wrapper, and interpreters that
/// run `ln` themselves.
#[test]
fn issue_623_finds_links_behind_keywords_wrappers_and_interpreter_commands() {
    for script in [
        "if [ -d .agents ]; then ln -s .agents ag; fi",
        "for f in x; do ln -s .agents ag; done",
        "(cd sub && ln -s ../.agents a)",
        "{ ln -s .agents ag; }",
        "x=$(ln -s .agents ag)",
        "sudo -u root ln -s .agents ag",
        "env -i PATH=/bin ln -s .agents ag",
        "nice -n5 ln -s .agents ag",
        "timeout 5 ln -s .agents ag",
        "doas ln -s .agents ag",
        "echo ag | xargs ln -s .agents",
        "find . -maxdepth 0 -exec ln -s .agents ag \\;",
        "! ln -s .agents ag",
    ] {
        assert!(
            targets(&shell(script))
                .iter()
                .any(|target| target.ends_with(".agents")),
            "{script}: {:?}",
            targets(&shell(script))
        );
    }
    assert_eq!(targets(&shell("ln -st links .agents")), vec![".agents"]);
    assert_eq!(
        link_targets(&shell("ln -sft links .agents"))[0].link_folders,
        vec!["links".to_string()]
    );
    assert_eq!(targets(&shell("ln -tlinks -s .codex")), vec![".codex"]);
    assert_eq!(
        targets(&shell("ln --target-directory=links -s .codex")),
        vec![".codex"]
    );
    for script in [
        "sudo python3 - <<'EOF'\nimport os\nos.symlink('.agents', 'ag')\nEOF",
        "/usr/bin/env python3 - <<EOF\nimport os\nos.symlink('.agents', 'ag')\nEOF",
        "uv run python - <<EOF\nimport os\nos.symlink('.agents', 'ag')\nEOF",
        "python3 -c \"import os; os.system('ln -s .agents ag')\"",
        "python3 -c \"import subprocess; subprocess.run(['ln', '-s', '.agents', 'ag'])\"",
        "node -e \"require('child_process').execSync('ln -s .agents ag')\"",
        "ruby -e \"require 'fileutils'; FileUtils.ln_s('.agents', 'ag')\"",
    ] {
        assert!(
            targets(&shell(script)).contains(&".agents".to_string()),
            "{script}: {:?}",
            targets(&shell(script))
        );
    }
    assert_eq!(
        targets(&argv(&[
            "cmd",
            "/c",
            "cd",
            "sub",
            "&&",
            "mklink",
            "/D",
            "ag",
            "..\\.agents"
        ])),
        vec!["..\\.agents"]
    );
}

#[test]
fn issue_623_only_calls_that_create_a_link_count() {
    for code in [
        "os.symlink('a', 'b')",
        "fs.symlinkSync('a', 'b')",
        "Path('b').symlink_to('a')",
        "os.link('a', 'b')",
        "symlink \"a\", \"b\"",
        "File.link('a', 'b')",
    ] {
        assert!(calls_link(code), "{code}");
    }
    for code in [
        "os.unlink('a')",
        "os.readlink('a')",
        "Path('a').is_symlink()",
        "os.path.islink('a')",
        "print('symlinks')",
    ] {
        assert!(!calls_link(code), "{code}");
    }
    assert_eq!(
        targets(&shell("python3 -c \"import os; os.unlink('.agents/x')\"")),
        Vec::<String>::new()
    );
}

#[test]
fn issue_623_reads_relative_targets_from_folders_the_script_changes_to() {
    let found = link_targets(&shell("cd sub && ln -s ../.agents x"));
    assert_eq!(found.len(), 1);
    assert!(
        found[0].link_folders.contains(&"sub".to_string()),
        "{found:?}"
    );
}

#[cfg(unix)]
struct Project {
    _home: tempfile::TempDir,
    _root: tempfile::TempDir,
    root: PathBuf,
    inputs: Inputs,
}

#[cfg(unix)]
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

#[cfg(unix)]
fn refused(project: &Project, command: &[String], cwd: &Path) -> Option<String> {
    project.inputs.protected_target(&link_targets(command), cwd)
}

#[cfg(unix)]
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
    assert_eq!(
        refused(&project, &shell("cd sub && ln -s ../.agents x"), root),
        Some("../.agents".to_string())
    );
    // `$PWD` is the shell's folder.
    assert_eq!(
        refused(&project, &shell("ln -s \"$PWD/.agents\" ag"), root),
        Some("$PWD/.agents".to_string())
    );
    assert_eq!(
        refused(&project, &shell("ln -s \"$(pwd)/.codex\" ag"), root),
        Some("$(pwd)/.codex".to_string())
    );
    // From `sub`, `evil/../.agents` reads as `sub/.agents`, but `evil`
    // links to `inner`, so the system reads the project's `.agents`.
    std::fs::create_dir_all(root.join("inner")).expect("inner");
    std::os::unix::fs::symlink(root.join("inner"), root.join("sub/evil")).expect("evil");
    assert_eq!(
        refused(
            &project,
            &shell("ln -s evil/../.agents x"),
            &root.join("sub")
        ),
        Some("evil/../.agents".to_string())
    );
    // A link to the Corbanu home is refused too.
    let home = format!("ln -s {} h", project.inputs.codex_home.display());
    assert!(refused(&project, &shell(&home), root).is_some());
}

#[cfg(unix)]
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
