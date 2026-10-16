use codex_protocol::ThreadId;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Read;
use std::os::unix::fs::DirEntryExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// Entries looked at per repository at most; past it the check says it
/// could not look everywhere.
const MAX_ENTRIES: usize = 50_000;
const MAX_DEPTH: usize = 16;
const MAX_REPOS: usize = 8;
const MAX_REPOS_PER_THREAD: usize = 16;
const MAX_TODO_BYTES: u64 = 4 << 20;
/// A `.git` file or `commondir` longer than this is not read.
const SMALL_FILE_BYTES: u64 = 64 << 10;
/// A folder changed this recently is listed again next time: a change in the
/// same clock tick would not move its modification time.
const RACY_NANOS: i128 = 2_000_000_000;
/// Not visited below a git folder: git replaces these files by renaming.
const SKIPPED: [&str; 3] = ["objects", "refs", "lfs"];
/// The steps a later `git rebase --continue` (or `cherry-pick`/`revert
/// --continue`) runs, in each git folder.
const TODOS: [&str; 3] = [
    "rebase-merge/git-rebase-todo",
    "rebase-apply/git-rebase-todo",
    "sequencer/todo",
];
/// Legacy remote definitions git still reads for a remote with no URL in
/// its config.
const LEGACY_REMOTES: [&str; 2] = ["remotes", "branches"];
const SHOWN_EXEC_LINES: usize = 3;
const SHOWN_CHARS: usize = 160;

/// A project's repository: the `.git` git finds for it and its git folders.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Repo {
    /// As found (a folder, a `gitdir:` file or a link).
    dot_git: PathBuf,
    /// Real paths: the folder, or a `gitdir:` file's folder and its common
    /// folder.
    git_dirs: Vec<PathBuf>,
}

impl Repo {
    /// Paths that, where an agent command may write them, mean it may write
    /// this repository's git folder.
    pub(crate) fn probes(&self) -> Vec<PathBuf> {
        let mut probes = vec![self.dot_git.join("COMMIT_EDITMSG")];
        for dir in &self.git_dirs {
            for name in ["COMMIT_EDITMSG", "logs/HEAD", TODOS[0], TODOS[2]] {
                probes.push(dir.join(name));
            }
        }
        probes
    }
}

/// The repositories `folders` are in (the nearest `.git` at or above each,
/// as git finds it).
pub(crate) fn repos(folders: &[&Path]) -> Vec<Repo> {
    let mut repos: Vec<Repo> = Vec::new();
    for folder in folders {
        let Some(dot_git) = folder
            .ancestors()
            .map(|folder| folder.join(".git"))
            .find(|dot_git| std::fs::symlink_metadata(dot_git).is_ok())
        else {
            continue;
        };
        if repos.iter().any(|repo| repo.dot_git == dot_git) {
            continue;
        }
        if let Some(repo) = repo_at(dot_git) {
            repos.push(repo);
        }
        if repos.len() >= MAX_REPOS {
            break;
        }
    }
    repos
}

fn repo_at(dot_git: PathBuf) -> Option<Repo> {
    let meta = std::fs::metadata(&dot_git).ok()?;
    let mut git_dirs = Vec::new();
    if meta.is_dir() {
        git_dirs.push(std::fs::canonicalize(&dot_git).ok()?);
    } else {
        let text = read_small(&dot_git, SMALL_FILE_BYTES)?;
        let work_tree = dot_git.parent()?;
        let dir = text.lines().find_map(|line| line.strip_prefix("gitdir:"))?;
        let git_dir = std::fs::canonicalize(work_tree.join(dir.trim())).ok()?;
        if let Some(common) = read_small(&git_dir.join("commondir"), SMALL_FILE_BYTES)
            .and_then(|common| std::fs::canonicalize(git_dir.join(common.trim())).ok())
        {
            git_dirs.push(common);
        }
        git_dirs.push(git_dir);
    }
    git_dirs.dedup();
    Some(Repo { dot_git, git_dirs })
}

/// Reads a small regular file without blocking on a FIFO or device.
fn read_small(path: &Path, max: u64) -> Option<String> {
    let mut bytes = Vec::new();
    let complete = read_regular(path, max, &mut bytes)?;
    complete.then(|| String::from_utf8_lossy(&bytes).into_owned())
}

/// Reads up to `max` bytes of the regular file at `path` (followed) into
/// `bytes`; whether that is all of it. `None` if it is not a readable
/// regular file.
fn read_regular(path: &Path, max: u64, bytes: &mut Vec<u8>) -> Option<bool> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    file.take(max).read_to_end(bytes).ok()?;
    Some(meta.len() <= max)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Dir,
    File,
    Link,
    Other,
}

impl Kind {
    fn of(file_type: std::fs::FileType) -> Self {
        if file_type.is_symlink() {
            Kind::Link
        } else if file_type.is_dir() {
            Kind::Dir
        } else if file_type.is_file() {
            Kind::File
        } else {
            Kind::Other
        }
    }
}

fn nanos(meta: &std::fs::Metadata) -> i128 {
    i128::from(meta.mtime()) * 1_000_000_000 + i128::from(meta.mtime_nsec())
}

fn now_nanos() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(/*default*/ 0, |since| since.as_nanos() as i128)
}

/// Where a link inside a git folder leads.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Link {
    raw: PathBuf,
    resolved: Option<PathBuf>,
    /// Outside the repository's git folders, to a folder, to a hard-linked
    /// file, or unknown: git would write elsewhere through it.
    escapes: bool,
}

impl Link {
    fn read(path: &Path, git_dirs: &[PathBuf]) -> Self {
        let Ok(raw) = std::fs::read_link(path) else {
            return Link {
                raw: PathBuf::new(),
                resolved: None,
                escapes: true,
            };
        };
        let full = match path.parent() {
            Some(parent) if raw.is_relative() => parent.join(&raw),
            _ => raw.clone(),
        };
        // A missing target is resolved through its folder: git would create
        // it there.
        let resolved = std::fs::canonicalize(&full).ok().or_else(|| {
            let name = full.file_name()?;
            std::fs::canonicalize(full.parent()?)
                .ok()
                .map(|parent| parent.join(name))
        });
        let escapes = match &resolved {
            None => true,
            Some(target) => {
                !git_dirs.iter().any(|dir| target.starts_with(dir))
                    || std::fs::metadata(target).is_ok_and(|meta| meta.is_dir() || meta.nlink() > 1)
            }
        };
        Link {
            raw,
            resolved,
            escapes,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    kind: Kind,
    dev: u64,
    ino: u64,
    nlink: u64,
    /// Length and modification time, kept current for legacy remote files.
    stamp: (u64, i128),
    link: Option<Link>,
}

impl Entry {
    fn stat(path: &Path, kind: Kind, ino: u64) -> Self {
        let meta = std::fs::symlink_metadata(path).ok();
        Entry {
            kind,
            dev: meta.as_ref().map_or(/*default*/ 0, MetadataExt::dev),
            ino: meta.as_ref().map_or(ino, MetadataExt::ino),
            nlink: meta.as_ref().map_or(/*default*/ 1, MetadataExt::nlink),
            stamp: meta
                .as_ref()
                .map_or(/*default*/ (0, 0), |meta| (meta.len(), nanos(meta))),
            link: None,
        }
    }
}

#[derive(Clone, Debug)]
struct Folder {
    dev: u64,
    ino: u64,
    mtime: i128,
    /// Listed long enough after its last change to trust its time.
    settled: bool,
    /// A legacy remote folder of a git folder.
    legacy: bool,
    entries: HashMap<OsString, Entry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Todo {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: i128,
    exec: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DotGit {
    Missing,
    Folder {
        dev: u64,
        ino: u64,
    },
    File {
        dev: u64,
        ino: u64,
        text: Option<String>,
    },
    Link {
        target: Option<PathBuf>,
    },
    Other,
}

impl DotGit {
    fn read(path: &Path) -> Self {
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            return DotGit::Missing;
        };
        match Kind::of(meta.file_type()) {
            Kind::Dir => DotGit::Folder {
                dev: meta.dev(),
                ino: meta.ino(),
            },
            Kind::File => DotGit::File {
                dev: meta.dev(),
                ino: meta.ino(),
                text: read_small(path, SMALL_FILE_BYTES),
            },
            Kind::Link => DotGit::Link {
                target: std::fs::read_link(path).ok(),
            },
            Kind::Other => DotGit::Other,
        }
    }
}

#[derive(Clone, Debug)]
struct Snapshot {
    folders: HashMap<PathBuf, Folder>,
    todos: HashMap<PathBuf, Todo>,
    dot_git: DotGit,
    complete: bool,
}

fn walk(repo: &Repo, prev: Option<&Snapshot>) -> Snapshot {
    let mut snapshot = Snapshot {
        folders: HashMap::new(),
        todos: HashMap::new(),
        dot_git: DotGit::read(&repo.dot_git),
        complete: true,
    };
    let mut roots = repo.git_dirs.clone();
    let mut count = 0;
    let mut stack: Vec<(PathBuf, usize, bool)> = repo
        .git_dirs
        .iter()
        .map(|dir| (dir.clone(), 0, false))
        .collect();
    while let Some((dir, depth, legacy)) = stack.pop() {
        if snapshot.folders.contains_key(&dir) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&dir) else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        let old = prev.and_then(|prev| prev.folders.get(&dir));
        let mut folder = match old.filter(|old| {
            old.settled
                && old.dev == meta.dev()
                && old.ino == meta.ino()
                && old.mtime == nanos(&meta)
        }) {
            Some(old) => old.clone(),
            None => match list(&dir, &meta, old) {
                Some(folder) => folder,
                None => {
                    snapshot.complete = false;
                    continue;
                }
            },
        };
        folder.legacy = legacy;
        for (name, entry) in &mut folder.entries {
            let path = dir.join(name);
            if entry.kind == Kind::Link {
                entry.link = Some(Link::read(&path, &repo.git_dirs));
            }
            if legacy && entry.kind != Kind::Dir {
                *entry = Entry {
                    link: entry.link.take(),
                    ..Entry::stat(&path, entry.kind, entry.ino)
                };
            }
        }
        count += folder.entries.len();
        if count > MAX_ENTRIES {
            snapshot.complete = false;
            break;
        }
        // A git folder: `HEAD` and objects or a common folder (a linked
        // worktree's). `logs` has a `HEAD` too, but neither.
        let has = |name: &str| folder.entries.contains_key(&OsString::from(name));
        let root = has("HEAD") && (has("objects") || has("commondir"));
        if root && !roots.contains(&dir) {
            roots.push(dir.clone());
        }
        for (name, entry) in &folder.entries {
            if entry.kind != Kind::Dir
                || (root && SKIPPED.iter().any(|skipped| name.as_os_str() == *skipped))
            {
                continue;
            }
            if depth >= MAX_DEPTH {
                snapshot.complete = false;
                continue;
            }
            let legacy = root && LEGACY_REMOTES.iter().any(|kept| name.as_os_str() == *kept);
            stack.push((dir.join(name), depth + 1, legacy));
        }
        snapshot.folders.insert(dir, folder);
    }
    for root in roots {
        for todo in TODOS {
            let path = root.join(todo);
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let old = prev
                .and_then(|prev| prev.todos.get(&path))
                .filter(|old| {
                    old.dev == meta.dev()
                        && old.ino == meta.ino()
                        && old.len == meta.len()
                        && old.mtime == nanos(&meta)
                        && now_nanos() - old.mtime > RACY_NANOS
                })
                .cloned();
            let todo = old.unwrap_or_else(|| Todo {
                dev: meta.dev(),
                ino: meta.ino(),
                len: meta.len(),
                mtime: nanos(&meta),
                exec: todo_exec_lines(&path),
            });
            snapshot.todos.insert(path, todo);
        }
    }
    snapshot
}

fn list(dir: &Path, meta: &std::fs::Metadata, old: Option<&Folder>) -> Option<Folder> {
    let mut entries = HashMap::new();
    // An unreadable folder fails the look (it can hide what git follows).
    for item in std::fs::read_dir(dir).ok()? {
        let Ok(item) = item else {
            continue;
        };
        let Ok(file_type) = item.file_type() else {
            continue;
        };
        let (name, kind, ino) = (item.file_name(), Kind::of(file_type), item.ino());
        let entry = old
            .and_then(|old| old.entries.get(&name))
            .filter(|old| old.kind == kind && old.ino == ino)
            .cloned()
            .unwrap_or_else(|| Entry::stat(&item.path(), kind, ino));
        entries.insert(name, entry);
    }
    let after = std::fs::symlink_metadata(dir).ok()?;
    let mtime = nanos(meta);
    Some(Folder {
        dev: meta.dev(),
        ino: meta.ino(),
        mtime,
        settled: nanos(&after) == mtime && now_nanos() - mtime > RACY_NANOS,
        legacy: false,
        entries,
    })
}

/// The `exec` lines of a rebase or sequencer todo file, as git reads them
/// (`exec` or `x`, after spaces or tabs). Fails closed: a file that cannot
/// be read or is too long counts as holding one.
fn todo_exec_lines(path: &Path) -> Vec<String> {
    let mut bytes = Vec::new();
    let Some(complete) = read_regular(path, MAX_TODO_BYTES, &mut bytes) else {
        return vec!["(the file can't be read)".to_string()];
    };
    let mut lines = exec_lines(&String::from_utf8_lossy(&bytes));
    if !complete && lines.is_empty() {
        lines.push("(the file is too long to check)".to_string());
    }
    lines
}

pub(super) fn exec_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.trim_start_matches([' ', '\t']).trim_end())
        .filter(|line| {
            let command = line.split([' ', '\t']).next().unwrap_or_default();
            (command == "exec" || command == "x") && line.len() > command.len()
        })
        .map(str::to_string)
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Finding {
    Link {
        path: PathBuf,
        target: Option<PathBuf>,
    },
    HardLink {
        path: PathBuf,
    },
    Exec {
        todo: PathBuf,
        lines: Vec<String>,
    },
    Remote {
        path: PathBuf,
    },
    Replaced {
        dot_git: PathBuf,
    },
    Incomplete {
        dot_git: PathBuf,
    },
}

impl Finding {
    /// What stays held until removed: the path and how to recognize it.
    fn held(&self) -> Option<Held> {
        let (path, follow) = match self {
            Finding::Link { path, .. } | Finding::HardLink { path } | Finding::Remote { path } => {
                (path, false)
            }
            Finding::Exec { todo, .. } => (todo, true),
            Finding::Replaced { .. } | Finding::Incomplete { .. } => return None,
        };
        Held::of(path, follow)
    }
}

fn compare(repo: &Repo, prev: &Snapshot, cur: &Snapshot) -> Vec<Finding> {
    let mut findings = Vec::new();
    if prev.dot_git != cur.dot_git {
        findings.push(Finding::Replaced {
            dot_git: repo.dot_git.clone(),
        });
    }
    let mut folders: Vec<_> = cur.folders.iter().collect();
    folders.sort_by(|a, b| a.0.cmp(b.0));
    for (dir, folder) in folders {
        let mut names: Vec<_> = folder.entries.iter().collect();
        names.sort_by(|a, b| a.0.cmp(b.0));
        for (name, entry) in names {
            let path = dir.join(name);
            let old = prev
                .folders
                .get(dir)
                .and_then(|folder| folder.entries.get(name));
            let new = old.is_none_or(|old| {
                old.kind != entry.kind || old.ino != entry.ino || old.dev != entry.dev
            });
            if let Some(link) = entry.link.as_ref().filter(|link| link.escapes)
                && (new || old.and_then(|old| old.link.as_ref()) != Some(link))
            {
                findings.push(Finding::Link {
                    path: path.clone(),
                    target: link.resolved.clone().or_else(|| Some(link.raw.clone())),
                });
            }
            if new && entry.kind == Kind::File && entry.nlink > 1 {
                findings.push(Finding::HardLink { path: path.clone() });
            }
            if folder.legacy
                && entry.kind != Kind::Dir
                && (new || old.is_some_and(|old| old.stamp != entry.stamp))
            {
                findings.push(Finding::Remote { path });
            }
        }
    }
    let mut todos: Vec<_> = cur.todos.iter().collect();
    todos.sort_by(|a, b| a.0.cmp(b.0));
    for (path, todo) in todos {
        if !todo.exec.is_empty() && prev.todos.get(path) != Some(todo) {
            findings.push(Finding::Exec {
                todo: path.clone(),
                lines: todo.exec.clone(),
            });
        }
    }
    findings
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Held {
    path: PathBuf,
    follow: bool,
    dev: u64,
    ino: u64,
}

impl Held {
    fn of(path: &Path, follow: bool) -> Option<Self> {
        let meta = if follow {
            std::fs::metadata(path)
        } else {
            std::fs::symlink_metadata(path)
        }
        .ok()?;
        Some(Held {
            path: path.to_path_buf(),
            follow,
            dev: meta.dev(),
            ino: meta.ino(),
        })
    }

    fn stands(&self) -> bool {
        Held::of(&self.path, self.follow).is_some_and(|now| now == *self)
    }
}

struct RepoState {
    repo: Repo,
    snapshot: Option<Snapshot>,
    incomplete_told: bool,
    used: u64,
}

#[derive(Default)]
struct ThreadState {
    repos: HashMap<PathBuf, Arc<Mutex<RepoState>>>,
    held: Vec<Held>,
    uses: u64,
}

static THREADS: LazyLock<Mutex<HashMap<ThreadId, ThreadState>>> = LazyLock::new(Default::default);

fn with_threads<T>(f: impl FnOnce(&mut HashMap<ThreadId, ThreadState>) -> T) -> T {
    f(&mut THREADS.lock().unwrap_or_else(PoisonError::into_inner))
}

fn state_of(thread: ThreadId, repo: &Repo) -> Arc<Mutex<RepoState>> {
    with_threads(|threads| {
        let state = threads.entry(thread).or_default();
        state.uses += 1;
        let used = state.uses;
        if !state.repos.contains_key(&repo.dot_git)
            && state.repos.len() >= MAX_REPOS_PER_THREAD
            && let Some(oldest) = state
                .repos
                .iter()
                .min_by_key(|(_, repo)| repo.lock().unwrap_or_else(PoisonError::into_inner).used)
                .map(|(path, _)| path.clone())
        {
            state.repos.remove(&oldest);
        }
        let entry = state
            .repos
            .entry(repo.dot_git.clone())
            .or_insert_with(|| {
                Arc::new(Mutex::new(RepoState {
                    repo: repo.clone(),
                    snapshot: None,
                    incomplete_told: false,
                    used,
                }))
            })
            .clone();
        let mut locked = entry.lock().unwrap_or_else(PoisonError::into_inner);
        locked.used = used;
        locked.repo = repo.clone();
        drop(locked);
        entry
    })
}

/// Before a command that may write these repositories' git folders: the
/// first look, where this session has none yet.
pub(crate) fn begin(thread: ThreadId, repos: &[Repo]) {
    for repo in repos {
        let state = state_of(thread, repo);
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.snapshot.is_none() {
            state.snapshot = Some(walk(repo, /*prev*/ None));
        }
    }
}

/// After such a command: what is new since the last look, as a message for
/// the person (`None` if nothing).
pub(crate) fn finish(thread: ThreadId, repos: &[Repo], level: &str) -> Option<String> {
    let findings = check(thread, repos);
    report(&findings, level)
}

/// At the end of a turn: the repositories this session looked at before.
pub(crate) fn recheck(thread: ThreadId, level: &str) -> Option<String> {
    let repos: Vec<Repo> = with_threads(|threads| {
        threads.get(&thread).map_or_else(Vec::new, |state| {
            state
                .repos
                .values()
                .map(|repo| {
                    repo.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .repo
                        .clone()
                })
                .collect()
        })
    });
    if repos.is_empty() {
        return None;
    }
    finish(thread, &repos, level)
}

pub(super) fn check(thread: ThreadId, repos: &[Repo]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for repo in repos {
        let state = state_of(thread, repo);
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        let prev = state.snapshot.take();
        let cur = walk(repo, prev.as_ref());
        if let Some(prev) = &prev {
            findings.extend(compare(repo, prev, &cur));
        }
        if !cur.complete && !state.incomplete_told {
            state.incomplete_told = true;
            findings.push(Finding::Incomplete {
                dot_git: repo.dot_git.clone(),
            });
        }
        state.snapshot = Some(cur);
    }
    let held: Vec<Held> = findings.iter().filter_map(Finding::held).collect();
    if !held.is_empty() {
        with_threads(|threads| {
            let state = threads.entry(thread).or_default();
            for held in held {
                if !state.held.contains(&held) {
                    state.held.push(held);
                }
            }
        });
    }
    findings
}

/// What this session found in a git folder that is still there: while any
/// is, a person's approval does not lift the protected-file rules.
pub(crate) fn held(thread: ThreadId) -> Vec<PathBuf> {
    let held: Vec<Held> = with_threads(|threads| {
        threads
            .get(&thread)
            .map_or_else(Vec::new, |state| state.held.clone())
    });
    if held.is_empty() {
        return Vec::new();
    }
    let standing: Vec<Held> = held.into_iter().filter(Held::stands).collect();
    with_threads(|threads| {
        if let Some(state) = threads.get_mut(&thread) {
            state.held.retain(|held| standing.contains(held));
        }
    });
    standing.into_iter().map(|held| held.path).collect()
}

/// Shown text: control characters replaced, long text cut.
fn shown(text: &str) -> String {
    let mut out: String = text
        .chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .take(SHOWN_CHARS)
        .collect();
    if text.chars().count() > SHOWN_CHARS {
        out.push('…');
    }
    out
}

fn shown_path(path: &Path) -> String {
    shown(&path.display().to_string())
}

/// `path` quoted for a POSIX shell.
fn quoted(path: &Path) -> String {
    format!("'{}'", shown_path(path).replace('\'', r"'\''"))
}

pub(super) fn report(findings: &[Finding], level: &str) -> Option<String> {
    if findings.is_empty() {
        return None;
    }
    let mut lines = vec![format!(
        "Security level {level}: agent commands could write this project's git folder, and \
         Corbanu found new things there that your own git would follow outside the sandbox. \
         Nothing was removed. Check them, and remove what you didn't make:"
    )];
    for finding in findings {
        lines.push(match finding {
            Finding::Link { path, target } => format!(
                "- {} is a link to {}: git writes through it (for example on your next commit, \
                 fetch or pull). Remove the link (its target stays): rm {}",
                shown_path(path),
                target
                    .as_deref()
                    .map_or_else(|| "a place that can't be resolved".to_string(), shown_path),
                quoted(path)
            ),
            Finding::HardLink { path } => format!(
                "- {} is a hard link: the same file has another name, maybe outside the project, \
                 and git writes into it. Remove this name: rm {}",
                shown_path(path),
                quoted(path)
            ),
            Finding::Exec { todo, lines } => {
                let state = todo.parent().unwrap_or(todo);
                let continues = if state.ends_with("sequencer") {
                    "git cherry-pick --continue or git revert --continue"
                } else {
                    "git rebase --continue"
                };
                let mut shown_lines: Vec<String> = lines
                    .iter()
                    .take(SHOWN_EXEC_LINES)
                    .map(|line| format!("\"{}\"", shown(line)))
                    .collect();
                if lines.len() > SHOWN_EXEC_LINES {
                    shown_lines.push(format!("and {} more", lines.len() - SHOWN_EXEC_LINES));
                }
                format!(
                    "- {} holds steps that run commands outside the sandbox if you continue \
                     ({continues}): {}. If you didn't start this yourself, remove it without \
                     continuing: rm -r {}",
                    shown_path(state),
                    shown_lines.join(", "),
                    quoted(state)
                )
            }
            Finding::Remote { path } => format!(
                "- {} defines a git remote, which git fetch, pull and push use for a remote with \
                 no URL in your git config. Remove it: rm {}",
                shown_path(path),
                quoted(path)
            ),
            Finding::Replaced { dot_git } => format!(
                "- {} was replaced. If you didn't do this, check its hooks folder and config \
                 before you run git in this project.",
                shown_path(dot_git)
            ),
            Finding::Incomplete { dot_git } => format!(
                "- {} couldn't be checked fully (too many entries, or a folder can't be read). \
                 Look for links in it before you run git in this project.",
                shown_path(dot_git)
            ),
        });
    }
    if findings.iter().any(|finding| finding.held().is_some()) {
        lines.push(format!(
            "Until these are removed, approving a command doesn't lift {level}'s protected-file \
             rules in this session."
        ));
    }
    Some(lines.join("\n"))
}
