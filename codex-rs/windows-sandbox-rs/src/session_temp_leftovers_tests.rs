use super::*;
use pretty_assertions::assert_eq;
use std::fs;
use tempfile::TempDir;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("create folder");
    fs::write(path, text).expect("write");
}

/// What `dir` holds that is named `name`, renamed aside as a code cache or
/// not: (still there, renamed aside).
fn where_is(dir: &Path, name: &str) -> (bool, Vec<PathBuf>) {
    let aside = fs::read_dir(dir)
        .expect("list")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name().is_some_and(|found| {
                found
                    .to_string_lossy()
                    .starts_with(&format!("{name}.corbanu-sandbox-cache-"))
            })
        })
        .collect();
    (dir.join(name).exists(), aside)
}

/// D456: at the D377 upgrade step, the code caches other programs keep in
/// the temp folder and run code from are renamed in place (moved, not
/// deleted), so those programs make new ones; folders not positively
/// identified as one stay.
#[test]
fn sec_win_d456_code_caches_in_temp_are_renamed_aside() {
    let temp = TempDir::new().expect("temp");
    let base = dunce::canonicalize(temp.path()).expect("canonical");
    // Code caches.
    write(
        &base.join("node-compile-cache\\v22.12.0-x64-1a2b-0\\c.cache"),
        "node",
    );
    write(&base.join("v8-compile-cache-0\\12.4\\x.BLOB"), "v8");
    write(&base.join("node-jiti\\a.js"), "jiti");
    write(&base.join("proj\\__pycache__\\m.cpython-312.pyc"), "pyc");
    write(&base.join("proj\\m.py"), "print(1)\n");
    // Not: Python's folder name with other files in it, a cache name below
    // the top, and a file with a cache's name.
    write(&base.join("other\\__pycache__\\notes.txt"), "keep");
    write(&base.join("sub\\node-compile-cache\\c.cache"), "keep");
    write(&base.join("jiti"), "a file");

    // An account that owns nothing here: only the code caches change.
    let found =
        take_sandbox_leftovers(&base, &["S-1-5-21-1-2-3-4".to_string()]).expect("take leftovers");
    println!("d456: {found:?}");
    assert_eq!((found.code_caches, found.code_caches_set_aside), (4, 4));

    let moved = |dir: &Path, name: &str, inner: &str, text: &str| {
        let (still_there, aside) = where_is(dir, name);
        assert!(!still_there, "{name} was not renamed aside");
        assert_eq!(aside.len(), 1, "{name}: {aside:?}");
        assert_eq!(
            fs::read_to_string(aside[0].join(inner)).expect("read"),
            text,
            "{name}"
        );
    };
    moved(
        &base,
        "node-compile-cache",
        "v22.12.0-x64-1a2b-0\\c.cache",
        "node",
    );
    moved(&base, "v8-compile-cache-0", "12.4\\x.BLOB", "v8");
    moved(&base, "node-jiti", "a.js", "jiti");
    moved(
        &base.join("proj"),
        "__pycache__",
        "m.cpython-312.pyc",
        "pyc",
    );
    for (path, text) in [
        ("proj\\m.py", "print(1)\n"),
        ("other\\__pycache__\\notes.txt", "keep"),
        ("sub\\node-compile-cache\\c.cache", "keep"),
        ("jiti", "a file"),
    ] {
        assert_eq!(
            fs::read_to_string(base.join(path)).expect("read"),
            text,
            "{path}"
        );
    }
    assert_eq!(
        where_is(&base.join("other"), "__pycache__").1,
        Vec::<PathBuf>::new()
    );
}
