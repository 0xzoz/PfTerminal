use super::unlabelled_writable_root_warnings;
use codex_protocol::ThreadId;
use codex_protocol::permissions::FileSystemAccessMode;
use codex_protocol::permissions::FileSystemPath;
use codex_protocol::permissions::FileSystemSandboxEntry;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_utils_absolute_path::AbsolutePathBuf;
use pretty_assertions::assert_eq;
use std::path::Path;

fn writable(paths: &[&Path]) -> FileSystemSandboxPolicy {
    FileSystemSandboxPolicy::restricted(
        paths
            .iter()
            .map(|path| {
                FileSystemSandboxEntry::new(
                    FileSystemPath::Path {
                        path: AbsolutePathBuf::from_absolute_path(path).expect("absolute"),
                    },
                    FileSystemAccessMode::Write,
                )
            })
            .collect(),
    )
}

/// #734: a writable root the non-admin sandbox couldn't label is named once
/// per thread, with the shared warning text.
#[test]
fn an_unlabelled_writable_root_is_warned_once_per_thread() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dunce::canonicalize(dir.path()).expect("canonical");
    let policy = writable(&[&root]);
    let thread = ThreadId::new();
    let expected = vec![codex_windows_sandbox::unlabelled_writable_root_warning(
        &root,
    )];
    let unlabelled = [root.clone()];
    assert_eq!(
        unlabelled_writable_root_warnings(thread, &policy, &root, &unlabelled),
        expected
    );
    assert_eq!(
        unlabelled_writable_root_warnings(thread, &policy, &root, &unlabelled),
        Vec::<String>::new()
    );
    assert_eq!(
        unlabelled_writable_root_warnings(ThreadId::new(), &policy, &root, &unlabelled),
        expected
    );
}

/// A root another session couldn't label, which isn't writable here, says
/// nothing in this thread.
#[test]
fn an_unlabelled_root_outside_this_runs_writable_roots_is_not_warned() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dunce::canonicalize(dir.path()).expect("canonical");
    let mine = base.join("mine");
    let other = base.join("other");
    std::fs::create_dir_all(&mine).expect("mine");
    std::fs::create_dir_all(&other).expect("other");
    let policy = writable(&[&mine]);
    assert_eq!(
        unlabelled_writable_root_warnings(ThreadId::new(), &policy, &mine, &[other]),
        Vec::<String>::new()
    );
    assert_eq!(
        unlabelled_writable_root_warnings(ThreadId::new(), &policy, &mine, &[]),
        Vec::<String>::new()
    );
}
