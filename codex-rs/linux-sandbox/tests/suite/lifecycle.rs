//! #935 (Travis, 2026-10-19, D35 option A): a sandboxed command cannot
//! outlive its session. When the helper's parent exits, when the helper is
//! killed, when bubblewrap dies early or when the command exits leaving a
//! daemon behind, every process in the sandbox stops, double-forked daemons
//! included.
//!
//! Each command holds the helper's stdout open in every process it starts,
//! so the pipe's end of file means none is left. The cases where
//! bubblewrap dies before binding the sandbox's PID 1 to its parent (the
//! race in #935) run a real bubblewrap through a wrapper that drops
//! `--die-with-parent`, which leaves the sandbox in that state for good.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]

use codex_protocol::models::PermissionProfile;
use codex_protocol::permissions::FileSystemAccessMode;
use codex_protocol::permissions::FileSystemPath;
use codex_protocol::permissions::FileSystemSandboxEntry;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::FileSystemSpecialPath;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_sandboxing::landlock::create_linux_sandbox_command_args_for_permission_profile;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::io::BufRead;
use std::io::BufReader;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::ChildStdout;
use std::process::Command;
use std::process::Stdio;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;
use tempfile::TempDir;

/// How long the sandbox may take to start, and to be gone once stopped.
const BOUND: Duration = Duration::from_secs(20);

fn helper() -> PathBuf {
    let helper = PathBuf::from(env!("CARGO_BIN_EXE_codex-linux-sandbox"));
    helper.canonicalize().unwrap_or(helper)
}

/// The system bubblewrap, which the wrapper runs.
fn system_bwrap() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join("bwrap"))
        .find(|path| path.is_file())
}

/// A unique `sleep` duration, to find the process by its command line.
fn marker() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!(
        "86400.{}{:04}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    )
}

/// Pids of processes whose command line contains `marker`.
fn processes_with(marker: &str) -> Vec<i32> {
    let mut pids = Vec::new();
    for entry in std::fs::read_dir("/proc").unwrap().flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse().ok()) else {
            continue;
        };
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        if String::from_utf8_lossy(&cmdline).contains(marker) {
            pids.push(pid);
        }
    }
    pids
}

fn kill(pid: i32) {
    // SAFETY: plain syscall.
    unsafe {
        libc::kill(pid, libc::SIGKILL);
    }
}

/// Writes an executable file from another process (#635): a descriptor
/// open for writing here would leak into other tests' forks and make
/// executing the file fail with `ETXTBSY`.
fn write_executable(path: &Path, contents: &str) {
    let status = Command::new("/bin/sh")
        .args([
            "-c",
            "printf '%s' \"$1\" > \"$2\" && chmod 755 \"$2\"",
            "sh",
        ])
        .arg(contents)
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success(), "write {}", path.display());
}

/// How bubblewrap runs.
#[derive(Clone, Copy, PartialEq)]
enum Bwrap {
    /// As the helper finds it.
    Real,
    /// Without `--die-with-parent`: PID 1 never binds to its parent.
    Unbound,
}

struct Fixture {
    work: TempDir,
    wrapper_dir: TempDir,
    /// The command's daemon and foreground `sleep` markers.
    daemon: String,
    foreground: String,
}

impl Fixture {
    fn new() -> Self {
        Self {
            work: TempDir::new().unwrap(),
            wrapper_dir: TempDir::new().unwrap(),
            daemon: marker(),
            foreground: marker(),
        }
    }

    fn pid_file(&self) -> PathBuf {
        self.wrapper_dir.path().join("bwrap-pids")
    }

    /// The helper's arguments for `script`, writable in the work folder.
    fn helper_args(&self, script: &str) -> Vec<String> {
        let work = self.work.path().canonicalize().unwrap();
        let profile = PermissionProfile::from_runtime_permissions(
            &FileSystemSandboxPolicy::restricted(vec![
                FileSystemSandboxEntry::new(
                    FileSystemPath::Special {
                        value: FileSystemSpecialPath::Root,
                    },
                    FileSystemAccessMode::Read,
                ),
                FileSystemSandboxEntry::new(
                    FileSystemPath::Path {
                        path: AbsolutePathBuf::try_from(work.as_path()).unwrap(),
                    },
                    FileSystemAccessMode::Write,
                ),
            ]),
            NetworkSandboxPolicy::Restricted,
        );
        create_linux_sandbox_command_args_for_permission_profile(
            vec!["/bin/sh".to_string(), "-c".to_string(), script.to_string()],
            &work,
            &profile,
            &work,
            /*use_legacy_landlock*/ false,
            /*allow_network_for_proxy*/ false,
        )
    }

    /// `program` with `args`, finding bubblewrap as `bwrap` says.
    fn command(&self, bwrap: Bwrap, program: &Path, args: &[String]) -> Command {
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(self.work.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if bwrap == Bwrap::Unbound {
            let real = system_bwrap().unwrap();
            let wrapper = self.wrapper_dir.path().join("bwrap");
            write_executable(
                &wrapper,
                &format!(
                    "#!/bin/sh\necho $$ >> '{}'\nfor arg do\n  shift\n  [ \"$arg\" = --die-with-parent ] && continue\n  set -- \"$@\" \"$arg\"\ndone\nexec '{}' \"$@\"\n",
                    self.pid_file().display(),
                    real.display()
                ),
            );
            let path = std::env::var_os("PATH").unwrap_or_default();
            let mut dirs = vec![self.wrapper_dir.path().to_path_buf()];
            dirs.extend(std::env::split_paths(&path));
            command.env("PATH", std::env::join_paths(dirs).unwrap());
        }
        command
    }

    /// A command that leaves a double-forked daemon, says `ready`, then
    /// sleeps in the foreground (or exits, without `foreground`).
    fn script(&self, foreground: bool) -> String {
        let mut script = format!("(setsid sleep {} &); echo ready", self.daemon);
        if foreground {
            script.push_str(&format!("; exec sleep {}", self.foreground));
        }
        script
    }

    /// Whether the sandbox runs here; `false` skips the test.
    fn sandbox_runs(&self, bwrap: Bwrap) -> bool {
        if bwrap == Bwrap::Unbound && system_bwrap().is_none() {
            eprintln!("skipping: no system bubblewrap");
            return false;
        }
        let output = self
            .command(bwrap, &helper(), &self.helper_args("echo ok"))
            .output()
            .unwrap();
        let runs = output.status.success() && String::from_utf8_lossy(&output.stdout) == "ok\n";
        if !runs {
            eprintln!("skipping: the sandbox does not run here ({output:?})");
            return false;
        }
        // Otherwise the case would test nothing.
        assert!(
            bwrap == Bwrap::Real || self.pid_file().is_file(),
            "the helper did not run bubblewrap through the wrapper"
        );
        true
    }

    /// Fails unless every process holding `stdout` is gone within
    /// [`BOUND`] and no marked process is left; kills what is left either
    /// way.
    fn assert_all_gone(&self, ended: &mpsc::Receiver<()>, what: &str) {
        let gone = ended.recv_timeout(BOUND).is_ok();
        let mut left: Vec<i32> = [&self.daemon, &self.foreground]
            .iter()
            .flat_map(|marker| processes_with(marker))
            .collect();
        left.sort_unstable();
        left.dedup();
        for pid in &left {
            kill(*pid);
        }
        assert!(
            gone && left.is_empty(),
            "{what}: sandboxed processes outlived it (stdout closed: {gone}, marked processes left: {left:?})"
        );
    }
}

/// Waits for `ready`, then signals when `stdout` reaches end of file.
fn watch(stdout: ChildStdout) -> mpsc::Receiver<()> {
    let (ready_tx, ready_rx) = mpsc::channel();
    let (ended_tx, ended_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut lines = BufReader::new(stdout).lines();
        if matches!(lines.next(), Some(Ok(line)) if line == "ready") {
            let _ = ready_tx.send(());
        }
        for _ in lines.by_ref() {}
        let _ = ended_tx.send(());
    });
    ready_rx
        .recv_timeout(BOUND)
        .expect("the sandboxed command did not start");
    ended_rx
}

fn reap(child: &mut Child) -> std::process::ExitStatus {
    child.wait().unwrap()
}

/// The helper's parent exits (the session's process dies): with a real
/// bubblewrap, the helper notices and stops the sandbox.
#[test]
fn the_sandbox_stops_when_the_helpers_parent_exits() {
    let fixture = Fixture::new();
    if !fixture.sandbox_runs(Bwrap::Real) {
        return;
    }
    let mut args = vec![
        "-c".to_string(),
        "\"$@\" & wait".to_string(),
        "sh".to_string(),
        helper().display().to_string(),
    ];
    args.extend(fixture.helper_args(&fixture.script(/*foreground*/ true)));
    let mut parent = fixture
        .command(Bwrap::Real, Path::new("/bin/sh"), &args)
        .spawn()
        .unwrap();
    let ended = watch(parent.stdout.take().unwrap());
    parent.kill().unwrap();
    reap(&mut parent);
    fixture.assert_all_gone(&ended, "the helper's parent exited");
}

/// The helper is killed (as a session's process-group kill does) while
/// bubblewrap has not bound the sandbox to itself.
#[test]
fn the_sandbox_stops_when_the_helper_is_killed() {
    let fixture = Fixture::new();
    if !fixture.sandbox_runs(Bwrap::Unbound) {
        return;
    }
    let mut helper = fixture
        .command(
            Bwrap::Unbound,
            &helper(),
            &fixture.helper_args(&fixture.script(/*foreground*/ true)),
        )
        .spawn()
        .unwrap();
    let ended = watch(helper.stdout.take().unwrap());
    helper.kill().unwrap();
    reap(&mut helper);
    fixture.assert_all_gone(&ended, "the helper was killed");
}

/// Bubblewrap dies before binding the sandbox to itself: the helper stops
/// the sandbox and exits as bubblewrap did.
#[test]
fn the_sandbox_stops_when_bubblewrap_dies_early() {
    use std::os::unix::process::ExitStatusExt;

    let fixture = Fixture::new();
    if !fixture.sandbox_runs(Bwrap::Unbound) {
        return;
    }
    std::fs::remove_file(fixture.pid_file()).unwrap();
    let mut helper = fixture
        .command(
            Bwrap::Unbound,
            &helper(),
            &fixture.helper_args(&fixture.script(/*foreground*/ true)),
        )
        .spawn()
        .unwrap();
    let ended = watch(helper.stdout.take().unwrap());
    // The last bubblewrap started is the command's (probes run first).
    let pids = std::fs::read_to_string(fixture.pid_file()).unwrap();
    kill(pids.lines().last().unwrap().parse().unwrap());
    fixture.assert_all_gone(&ended, "bubblewrap died");
    assert_eq!(reap(&mut helper).signal(), Some(libc::SIGKILL));
}

/// The command exits and leaves a daemon: the daemon is stopped and the
/// helper exits with the command's status, also when bubblewrap never
/// bound the sandbox to itself.
#[test]
fn a_daemon_left_by_a_finished_command_is_stopped() {
    for bwrap in [Bwrap::Real, Bwrap::Unbound] {
        let fixture = Fixture::new();
        if !fixture.sandbox_runs(bwrap) {
            return;
        }
        let script = format!("{}; exit 7", fixture.script(/*foreground*/ false));
        let mut helper = fixture
            .command(bwrap, &helper(), &fixture.helper_args(&script))
            .spawn()
            .unwrap();
        let ended = watch(helper.stdout.take().unwrap());
        let what = match bwrap {
            Bwrap::Real => "the command exited",
            Bwrap::Unbound => "the command exited (unbound sandbox)",
        };
        fixture.assert_all_gone(&ended, what);
        assert_eq!(reap(&mut helper).code(), Some(7));
    }
}
