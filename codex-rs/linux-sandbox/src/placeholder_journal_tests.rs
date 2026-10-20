use super::*;
use pretty_assertions::assert_eq;
use std::os::unix::fs::PermissionsExt;

struct Setup {
    _dir: tempfile::TempDir,
    base: PathBuf,
    journal: Journal,
}

impl Setup {
    fn new() -> Self {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let base = dir.path().canonicalize().expect("canonical");
        std::fs::create_dir(base.join("home")).expect("home");
        std::fs::create_dir(base.join("corbanu")).expect("corbanu home");
        let journal =
            open_journal(&base.join("corbanu/sandbox-placeholders")).expect("record folder");
        Self {
            _dir: dir,
            base,
            journal,
        }
    }

    fn records(&self) -> Vec<String> {
        let mut names = self
            .journal
            .list()
            .expect("list")
            .into_iter()
            .filter(|name| name.ends_with(".rec"))
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    fn report(&self) -> String {
        std::fs::read_to_string(self.base.join("corbanu/sandbox-placeholders/kept.log"))
            .unwrap_or_default()
    }

    /// Makes a placeholder at `path` as a run does, and writes the record of
    /// a run (`owner`) that made it.
    fn made_by(&self, owner: Owner, path: &Path, kind: Kind) -> Entry {
        let identity = make_placeholder(path, kind).expect("placeholder");
        let entry = Entry::Made {
            kind,
            path: path.to_path_buf(),
            real: path.to_path_buf(),
            identity,
        };
        self.write(owner, vec![entry.clone()]);
        entry
    }

    fn write(&self, owner: Owner, entries: Vec<Entry>) {
        let stem = format!("{}-{}-1", owner.pid, owner.start.unwrap_or(0));
        self.journal
            .write_named(&stem, &Record { owner, entries })
            .expect("record");
    }

    fn sweep(&mut self) {
        self.journal.sweep(&|_, _| false);
    }
}

/// A run that has ended: its pid is gone (or names another process, which
/// started at another time).
fn ended() -> Owner {
    let mut child = std::process::Command::new("true").spawn().expect("spawn");
    child.wait().expect("wait");
    Owner {
        host: own_host().cloned(),
        boot: own_boot().cloned(),
        pid: child.id() as libc::pid_t,
        start: Some(1),
        pidns: crate::placeholder_registry::own_pid_namespace(),
    }
}

fn this_run() -> Owner {
    own_owner()
}

/// Waits past the file system's timestamp granularity, so a file made now
/// cannot have the times of one made just before.
fn tick() {
    std::thread::sleep(Duration::from_millis(50));
}

#[test]
fn a_record_round_trips_and_one_not_written_by_this_version_is_not_read() {
    let record = Record {
        owner: Owner {
            host: Some("h1".to_string()),
            boot: Some("b1".to_string()),
            pid: 42,
            start: Some(7),
            pidns: None,
        },
        entries: vec![
            Entry::Uses {
                path: PathBuf::from("/a/b c"),
                real: PathBuf::from("/real/b c"),
            },
            Entry::Pending {
                kind: Kind::Directory,
                path: PathBuf::from("/a/.git"),
                real: PathBuf::from("/a/.git"),
            },
            Entry::Made {
                kind: Kind::File,
                path: PathBuf::from("/home/u/.pfterminal"),
                real: PathBuf::from("/home/u/.pfterminal"),
                identity: Identity {
                    dev: 1,
                    ino: 2,
                    mode: 0o100444,
                    uid: 1000,
                    size: 0,
                    mtime: (3, 4),
                    ctime: (5, 6),
                },
            },
        ],
    };
    let text = serialize(&record);
    assert_eq!(parse(&text), Some(record));

    let made = text
        .lines()
        .find(|line| line.starts_with("made "))
        .expect("made line")
        .to_string();
    for changed in [
        text.replace(HEADER, "corbanu-sandbox-placeholders 2"),
        text.replace("end\n", ""),
        format!("{text}uses 2f 2f\n"),
        text.replace(&made, &format!("{made} 9")),
        // A relative path, a path with `..`, and two names.
        text.replace(&hex(Path::new("/a/.git")), &hex(Path::new("a/.git"))),
        text.replace(&hex(Path::new("/a/.git")), &hex(Path::new("/a/../.git"))),
        text.replace(
            &format!(
                "{} {}",
                hex(Path::new("/a/.git")),
                hex(Path::new("/a/.git"))
            ),
            &format!(
                "{} {}",
                hex(Path::new("/a/.git")),
                hex(Path::new("/a/.svn"))
            ),
        ),
        text.replace("pid 42", "pid x"),
    ] {
        assert_eq!(parse(&changed), None, "{changed}");
    }
}

#[test]
fn a_run_is_live_until_it_ends_its_boot_ends_or_its_pid_is_reused() {
    assert!(owner_is_live(&this_run()));
    assert!(!owner_is_live(&ended()));
    let another_boot = Owner {
        boot: Some("another-boot".to_string()),
        ..this_run()
    };
    assert!(!owner_is_live(&another_boot));
    let reused_pid = Owner {
        start: Some(1),
        ..this_run()
    };
    assert!(!owner_is_live(&reused_pid));
    // A run on another machine sharing the home is taken as running.
    let other_machine = Owner {
        host: Some("another-machine".to_string()),
        boot: Some("another-boot".to_string()),
        ..ended()
    };
    assert!(owner_is_live(&other_machine));
    // One whose PID namespace is not known is taken as running.
    let unknown_namespace = Owner {
        pidns: None,
        ..ended()
    };
    assert!(owner_is_live(&unknown_namespace));
    // A run in another PID namespace is taken as running.
    let other_namespace = Owner {
        pidns: Some(1),
        ..ended()
    };
    assert!(owner_is_live(&other_namespace));
}

#[test]
fn a_placeholder_a_run_made_and_left_is_removed() {
    let mut setup = Setup::new();
    let file = setup.base.join("home/.pfterminal");
    let folder = setup.base.join("home/.git");
    setup.made_by(ended(), &file, Kind::File);
    setup.made_by(
        Owner {
            boot: Some("another-boot".to_string()),
            ..this_run()
        },
        &folder,
        Kind::Directory,
    );
    let status = std::fs::symlink_metadata(&file).expect("made");
    assert!(status.is_file() && status.len() == 0);
    assert_eq!(status.permissions().mode() & 0o777, 0o444);
    assert_eq!(setup.records().len(), 2);

    setup.sweep();

    assert!(std::fs::symlink_metadata(&file).is_err());
    assert!(std::fs::symlink_metadata(&folder).is_err());
    assert_eq!(setup.records(), Vec::<String>::new());
    assert_eq!(setup.report(), "");
}

#[test]
fn a_persons_own_empty_read_only_file_at_that_path_is_kept() {
    let mut setup = Setup::new();
    let path = setup.base.join("home/.pfterminal");
    setup.made_by(ended(), &path, Kind::File);
    std::fs::remove_file(&path).expect("the person removes it");
    tick();
    std::fs::write(&path, "").expect("and makes their own");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).expect("read-only");
    let own = std::fs::symlink_metadata(&path).expect("own").ino();

    setup.sweep();

    assert_eq!(std::fs::symlink_metadata(&path).expect("kept").ino(), own);
    assert_eq!(setup.records(), Vec::<String>::new(), "reported once");
    assert!(setup.report().contains(".pfterminal"), "{}", setup.report());
}

#[test]
fn a_placeholder_filled_replaced_or_changed_since_is_kept() {
    let mut setup = Setup::new();
    let filled = setup.base.join("home/filled");
    let replaced = setup.base.join("home/replaced");
    let changed = setup.base.join("home/changed");
    let folder = setup.base.join("home/folder");
    for path in [&filled, &replaced, &changed] {
        setup.made_by(ended(), path, Kind::File);
    }
    setup.made_by(ended(), &folder, Kind::Directory);
    tick();
    std::fs::set_permissions(&filled, std::fs::Permissions::from_mode(0o644)).expect("writable");
    std::fs::write(&filled, "data").expect("filled");
    let other = setup.base.join("home/other");
    std::fs::write(&other, "").expect("other");
    std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o444)).expect("read-only");
    std::fs::rename(&other, &replaced).expect("replaced");
    std::fs::set_permissions(&changed, std::fs::Permissions::from_mode(0o400)).expect("changed");
    std::fs::write(folder.join("kept"), "x").expect("filled folder");

    setup.sweep();

    assert_eq!(std::fs::read_to_string(&filled).expect("kept"), "data");
    assert!(replaced.is_file() && changed.is_file() && folder.join("kept").is_file());
    assert_eq!(setup.records(), Vec::<String>::new());
    let report = setup.report();
    for name in ["filled", "replaced", "changed", "folder"] {
        assert!(
            report.contains(&format!("home/{name}:")),
            "{name}: {report}"
        );
    }
}

#[test]
fn a_placeholder_a_live_run_uses_is_kept_until_it_ends() {
    let mut setup = Setup::new();
    let path = setup.base.join("home/.pfterminal");
    let inside = setup.base.join("home/.git");
    let maker = ended();
    setup.made_by(maker.clone(), &path, Kind::File);
    setup.made_by(
        Owner {
            pid: maker.pid,
            start: Some(2),
            ..maker
        },
        &inside,
        Kind::Directory,
    );
    // A live run mounts over the file, and has a placeholder of its own in
    // the folder.
    let live = Owner {
        start: own_owner().start,
        ..this_run()
    };
    let live_stem = format!("{}-{}-1", live.pid, live.start.unwrap_or(0));
    setup.write(
        live,
        vec![
            Entry::Uses {
                path: path.clone(),
                real: path.clone(),
            },
            Entry::Uses {
                path: inside.join("hooks"),
                real: inside.join("hooks"),
            },
        ],
    );

    setup.sweep();
    assert!(path.is_file() && inside.is_dir(), "kept while used");
    assert_eq!(setup.records().len(), 3);

    setup
        .journal
        .unlink(&format!("{live_stem}.rec"))
        .expect("the live run ends");
    setup.sweep();
    assert!(std::fs::symlink_metadata(&path).is_err());
    assert!(std::fs::symlink_metadata(&inside).is_err());
    assert_eq!(setup.records(), Vec::<String>::new());

    // A live run of the temporary folder's registry keeps it too.
    setup.made_by(ended(), &path, Kind::File);
    setup.journal.sweep(&|spelled, _| spelled == path);
    assert!(path.is_file());
    assert_eq!(setup.records().len(), 1);
}

#[test]
fn a_pending_placeholder_is_never_removed() {
    let mut setup = Setup::new();
    let path = setup.base.join("home/.pfterminal");
    std::fs::write(&path, "").expect("bubblewrap made it");
    setup.write(
        ended(),
        vec![Entry::Pending {
            kind: Kind::File,
            path: path.clone(),
            real: path.clone(),
        }],
    );

    setup.sweep();

    assert!(path.is_file());
    assert_eq!(setup.records(), Vec::<String>::new());
    assert!(setup.report().contains(".pfterminal"));
}

#[test]
fn removal_does_not_follow_a_link_above_the_placeholder() {
    let mut setup = Setup::new();
    let folder = setup.base.join("home/folder");
    std::fs::create_dir(&folder).expect("folder");
    let path = folder.join("x");
    setup.made_by(ended(), &path, Kind::File);
    // The folder moves, and a link to it takes its place: the placeholder,
    // as made, is reached through the link.
    let moved = setup.base.join("home/moved");
    std::fs::rename(&folder, &moved).expect("move");
    std::os::unix::fs::symlink(&moved, &folder).expect("link");
    assert!(
        std::fs::symlink_metadata(&path)
            .expect("through the link")
            .is_file()
    );

    setup.sweep();

    assert!(moved.join("x").is_file(), "not removed through the link");
    assert_eq!(setup.records(), Vec::<String>::new());
}

#[test]
fn a_run_records_and_makes_its_placeholders_then_finishes() {
    let mut setup = Setup::new();
    let file = setup.base.join("home/.pfterminal");
    let folder = setup.base.join("home/.git");
    let held = setup.base.join("home/held");
    std::fs::write(&held, "").expect("held");
    let planned = [
        Planned {
            kind: Kind::File,
            path: &file,
            real: &file,
            makes: true,
        },
        Planned {
            kind: Kind::Directory,
            path: &folder,
            real: &folder,
            makes: true,
        },
    ];
    setup
        .journal
        .register(&planned, &[(held.clone(), held.clone())]);

    assert!(file.is_file() && folder.is_dir());
    let records = setup.records();
    assert_eq!(records.len(), 1);
    let record = setup.journal.read(&records[0]).expect("record");
    assert_eq!(record.owner, own_owner());
    let made = record
        .entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Made { path, identity, .. } => Some((path.clone(), identity.ino)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        made,
        vec![
            (
                file.clone(),
                std::fs::symlink_metadata(&file).expect("file").ino()
            ),
            (
                folder.clone(),
                std::fs::symlink_metadata(&folder).expect("folder").ino()
            ),
        ]
    );
    assert!(record.entries.contains(&Entry::Uses {
        path: held.clone(),
        real: held.clone()
    }));
    // Another run's sweep leaves a live run's placeholders.
    let mut other = open_journal(&setup.base.join("corbanu/sandbox-placeholders")).expect("other");
    other.sweep(&|_, _| false);
    assert!(file.is_file() && folder.is_dir());

    // Once its command has ended it uses nothing: only what it made stays.
    setup.journal.leave();
    let record = setup.journal.read(&records[0]).expect("record");
    assert_eq!(record.entries.len(), 2);
    assert!(
        record
            .entries
            .iter()
            .all(|entry| matches!(entry, Entry::Made { .. }))
    );
    assert!(!uses(&record, &held, &held));

    // Its cleanup removed the file; another run still holds the folder.
    std::fs::remove_file(&file).expect("cleanup");
    setup.journal.finish();
    let records = setup.records();
    assert_eq!(records.len(), 1);
    let record = setup.journal.read(&records[0]).expect("record");
    assert!(matches!(
        record.entries.as_slice(),
        [Entry::Made { path, .. }] if *path == folder
    ));
    std::fs::remove_dir(&folder).expect("the holder removes it");
    setup.journal.finish();
    assert_eq!(setup.records(), Vec::<String>::new());

    // Never over something already there.
    std::fs::write(&file, "").expect("someone's file");
    setup.journal.register(&planned[..1], &[]);
    let records = setup.records();
    let record = setup.journal.read(&records[0]).expect("record");
    assert!(
        record
            .entries
            .iter()
            .all(|entry| !matches!(entry, Entry::Made { .. }))
    );
    setup.journal.finish();
    assert_eq!(setup.records(), Vec::<String>::new());
}

#[test]
fn a_record_folder_of_another_kind_is_not_used() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let base = dir.path().canonicalize().expect("canonical");
    // The home itself is never made.
    assert!(open_journal(&base.join("missing/sandbox-placeholders")).is_err());
    assert!(!base.join("missing").exists());
    // Not through a link.
    std::fs::create_dir(base.join("target")).expect("target");
    std::fs::create_dir(base.join("home")).expect("home");
    std::os::unix::fs::symlink(base.join("target"), base.join("home/sandbox-placeholders"))
        .expect("link");
    assert!(open_journal(&base.join("home/sandbox-placeholders")).is_err());
    // A folder others may write is made private.
    std::fs::create_dir(base.join("home/open")).expect("open");
    std::fs::set_permissions(
        base.join("home/open"),
        std::fs::Permissions::from_mode(0o777),
    )
    .expect("mode");
    open_journal(&base.join("home/open")).expect("journal");
    assert_eq!(
        std::fs::metadata(base.join("home/open"))
            .expect("open")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[test]
fn an_unreadable_record_of_a_live_run_keeps_every_placeholder() {
    let mut setup = Setup::new();
    let path = setup.base.join("home/.pfterminal");
    setup.made_by(ended(), &path, Kind::File);
    // A newer version's record of a run that is running.
    let live = this_run();
    let unreadable = setup.base.join(format!(
        "corbanu/sandbox-placeholders/{}-{}-9.rec",
        live.pid,
        live.start.unwrap_or(0)
    ));
    std::fs::write(&unreadable, "corbanu-sandbox-placeholders 2\n").expect("record");
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o600)).expect("mode");

    setup.sweep();
    assert!(path.is_file(), "kept while that run runs");
    assert!(used_by_live_run_in(&mut setup.journal, &path));

    // Once that run has ended, it no longer counts.
    let ended = ended();
    std::fs::rename(
        &unreadable,
        setup.base.join(format!(
            "corbanu/sandbox-placeholders/{}-1-9.rec",
            ended.pid
        )),
    )
    .expect("rename");
    assert!(!used_by_live_run_in(&mut setup.journal, &path));
    setup.sweep();
    assert!(std::fs::symlink_metadata(&path).is_err());
}

fn used_by_live_run_in(journal: &mut Journal, path: &Path) -> bool {
    let _lock = journal.lock().expect("lock");
    let (records, unreadable_live) = journal.other_records();
    unreadable_live
        || records
            .iter()
            .filter(|(_, record)| owner_is_live(&record.owner))
            .any(|(_, record)| uses(record, path, path))
}

#[test]
fn a_mount_point_in_any_of_this_users_namespaces_is_found() {
    assert_eq!(unescape_mount_point(br"/a\040b\134c"), b"/a b\\c".to_vec());
    assert_eq!(unescape_mount_point(br"/plain\x"), br"/plain\x".to_vec());
    // This process's own `/proc` is a mount point; a fresh folder is not.
    assert!(mounted_anywhere(&[Path::new("/proc")]));
    let dir = tempfile::TempDir::new().expect("tempdir");
    assert!(!mounted_anywhere(&[dir.path()]));
}
