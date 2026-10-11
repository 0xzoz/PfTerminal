use super::*;
use codex_protocol::permissions::FileSystemAccessMode;
use codex_protocol::permissions::FileSystemPath;
use codex_protocol::permissions::FileSystemSandboxEntry;
use codex_protocol::permissions::credential_file_globs;
use pretty_assertions::assert_eq;
use std::time::Duration;
use std::time::Instant;

fn deny_globs(globs: Vec<String>) -> FileSystemSandboxPolicy {
    FileSystemSandboxPolicy::restricted(
        globs
            .into_iter()
            .map(|pattern| {
                FileSystemSandboxEntry::new(
                    FileSystemPath::GlobPattern { pattern },
                    FileSystemAccessMode::Deny,
                )
            })
            .collect(),
    )
}

fn root(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().canonicalize().unwrap()
}

/// A child that only waits to be stopped.
fn sleeper() -> std::process::Child {
    std::process::Command::new("sleep").arg("30").spawn().unwrap()
}

/// Waits for `child` to exit, up to a few seconds; whether it was killed.
fn killed(child: &mut std::process::Child) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().unwrap() {
            return !status.success();
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
    false
}

#[test]
fn watch_roots_are_the_existing_outermost_credential_folders() {
    let dir = tempfile::tempdir().unwrap();
    let work = root(&dir);
    std::fs::create_dir_all(work.join("nested")).unwrap();
    let mut globs = credential_file_globs(&work.to_string_lossy());
    globs.extend(credential_file_globs(&work.join("nested").to_string_lossy()));
    globs.extend(credential_file_globs("/"));
    globs.extend(credential_file_globs(&work.join("missing").to_string_lossy()));
    globs.push(format!("{}/**/*.pem", work.display()));
    assert_eq!(
        credential_watch_roots(&deny_globs(globs), &work),
        vec![work.clone()]
    );
    assert_eq!(
        credential_watch_roots(&deny_globs(Vec::new()), &work),
        Vec::<PathBuf>::new()
    );
    assert!(
        CredentialWatcher::for_policy(&deny_globs(Vec::new()), &work)
            .unwrap()
            .is_none()
    );
}

/// A credential file created below the folder while the command runs, also
/// in a folder created after the watch started, stops the command.
#[test]
fn a_credential_file_created_meanwhile_stops_the_command() {
    for (folder, name) in [("", ".env"), ("app/deep", ".env.local"), ("x", ".envrc")] {
        let dir = tempfile::tempdir().unwrap();
        let work = root(&dir);
        let policy = deny_globs(credential_file_globs(&work.to_string_lossy()));
        let watcher = CredentialWatcher::for_policy(&policy, &work)
            .unwrap()
            .unwrap();
        let mut child = sleeper();
        let monitor = watcher.start(child.id() as libc::pid_t, Vec::new());
        std::fs::create_dir_all(work.join(folder)).unwrap();
        std::fs::write(work.join(folder).join(name), "API_KEY=dummy").unwrap();
        assert!(killed(&mut child), "{folder}/{name}");
        let found = monitor.stop().expect("stopped");
        assert!(found.ends_with(name), "{found}");
    }
}

/// A folder moved in with a credential file inside stops the command too.
#[test]
fn a_folder_moved_in_with_a_credential_file_stops_the_command() {
    let dir = tempfile::tempdir().unwrap();
    let work = root(&dir);
    std::fs::create_dir(work.join("in")).unwrap();
    let outside = tempfile::tempdir_in(&work.join("..")).unwrap();
    std::fs::create_dir_all(outside.path().join("pkg")).unwrap();
    std::fs::write(outside.path().join("pkg/.env"), "API_KEY=dummy").unwrap();
    let policy = deny_globs(credential_file_globs(&work.join("in").to_string_lossy()));
    let watcher = CredentialWatcher::for_policy(&policy, &work)
        .unwrap()
        .unwrap();
    let mut child = sleeper();
    let monitor = watcher.start(child.id() as libc::pid_t, Vec::new());
    std::fs::rename(outside.path().join("pkg"), work.join("in/pkg")).unwrap();
    assert!(killed(&mut child));
    assert!(monitor.stop().expect("stopped").ends_with("pkg/.env"));
}

/// Other names, and bubblewrap's own placeholders, do not stop it.
#[test]
fn other_names_and_placeholders_do_not_stop_the_command() {
    let dir = tempfile::tempdir().unwrap();
    let work = root(&dir);
    let policy = deny_globs(credential_file_globs(&work.to_string_lossy()));
    let watcher = CredentialWatcher::for_policy(&policy, &work)
        .unwrap()
        .unwrap();
    let mut child = sleeper();
    let monitor = watcher.start(child.id() as libc::pid_t, vec![work.join(".env")]);
    for name in ["env", "x.env", ".environment", ".env"] {
        std::fs::write(work.join(name), "").unwrap();
    }
    std::fs::create_dir(work.join("sub")).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(child.try_wait().unwrap(), None);
    assert_eq!(monitor.stop(), None);
    child.kill().unwrap();
    child.wait().unwrap();
}

/// What existing links with a credential name point to is reported, so the
/// sandbox can hide it (the glob expansion does not list links).
#[test]
fn link_targets_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let work = root(&dir);
    std::fs::create_dir_all(work.join("app")).unwrap();
    std::fs::write(work.join("shared.secret"), "API_KEY=dummy").unwrap();
    std::os::unix::fs::symlink("../shared.secret", work.join("app/.env")).unwrap();
    std::os::unix::fs::symlink(".", work.join(".env.dir")).unwrap();
    let policy = deny_globs(credential_file_globs(&work.to_string_lossy()));
    let watcher = CredentialWatcher::for_policy(&policy, &work)
        .unwrap()
        .unwrap();
    assert_eq!(watcher.link_targets(), [work.join("shared.secret")]);
}
