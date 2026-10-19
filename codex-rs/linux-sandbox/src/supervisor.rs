//! #935 (Travis, 2026-10-19, D35 option A): a bubblewrap sandbox cannot
//! outlive the helper that started it.
//!
//! bubblewrap binds the sandbox's PID 1 to its parent with
//! `PR_SET_PDEATHSIG` only after setting the sandbox up, and never checks
//! that the parent is still alive afterwards. When the outer bubblewrap dies
//! before then (the helper killed during setup, a credential stop, a crash),
//! PID 1 keeps running with PPID 1 in its own session, and so does the
//! command it starts.
//!
//! So the helper runs bubblewrap under a supervisor: a fork of the helper in
//! its own process group (out of reach of a kill aimed at the helper's), set
//! as a child subreaper. Every orphaned sandbox PID 1 becomes its child. It
//! stops the sandbox when bubblewrap exits or when its lifeline from the
//! helper closes (the helper exited or was killed, saw its own parent exit,
//! or stopped the command): it kills bubblewrap and every remaining child
//! with `SIGKILL`, which for a PID 1 takes its whole PID namespace with it
//! (double-forked daemons included), and reaps them all before exiting with
//! bubblewrap's status.

use std::os::fd::AsRawFd;
use std::os::fd::FromRawFd;
use std::os::fd::OwnedFd;
use std::os::fd::RawFd;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::thread;

/// The helper's end of the lifeline: the sandbox runs while it is open.
pub(crate) struct Lifeline {
    write: Mutex<Option<OwnedFd>>,
}

impl Lifeline {
    /// Stops the sandbox. Idempotent; the helper's exit does the same.
    pub(crate) fn cut(&self) {
        drop(
            self.write
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
    }
}

/// A lifeline pipe (both ends close-on-exec, so bubblewrap and the command
/// never hold it): the supervisor's read end and the helper's [`Lifeline`].
pub(crate) fn lifeline() -> (OwnedFd, Lifeline) {
    let mut fds = [-1; 2];
    // SAFETY: `fds` is a valid out array of two descriptors.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        let err = std::io::Error::last_os_error();
        panic!("failed to create the sandbox lifeline: {err}");
    }
    // SAFETY: both descriptors were just created and are owned here.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    (
        read,
        Lifeline {
            write: Mutex::new(Some(write)),
        },
    )
}

/// The helper's parent, opened while it is known to be alive.
pub(crate) enum ParentWatch {
    /// A pidfd: readable once the parent process has exited.
    Pidfd(OwnedFd),
    /// The parent exited before it could be opened.
    Gone,
    /// No pidfd (kernels before 5.3): `PR_SET_PDEATHSIG` delivers `SIGTERM`
    /// instead, which the helper forwards to bubblewrap.
    DeathSignal,
}

impl ParentWatch {
    /// Opens the helper's parent. Checks `getppid()` after opening (or
    /// after arming the death signal), so a parent that exits in between is
    /// never missed.
    pub(crate) fn open() -> Self {
        // SAFETY: plain syscalls.
        let parent = unsafe { libc::getppid() };
        let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, parent, 0) };
        let watch = if pidfd >= 0 {
            // SAFETY: the descriptor was just returned and is owned here.
            Self::Pidfd(unsafe { OwnedFd::from_raw_fd(pidfd as RawFd) })
        } else if std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            Self::Gone
        } else {
            // SAFETY: plain syscall on this process.
            if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) } < 0 {
                let err = std::io::Error::last_os_error();
                panic!("failed to watch the sandbox helper's parent: {err}");
            }
            Self::DeathSignal
        };
        // SAFETY: plain syscall.
        if unsafe { libc::getppid() } != parent {
            return Self::Gone;
        }
        watch
    }

    /// Cuts `lifeline` once the parent has exited. Call after forking the
    /// supervisor: this may start a thread.
    pub(crate) fn cut_when_gone(self, lifeline: Arc<Lifeline>) {
        match self {
            Self::Gone => lifeline.cut(),
            Self::DeathSignal => {}
            Self::Pidfd(pidfd) => {
                thread::spawn(move || {
                    let mut poll_fd = libc::pollfd {
                        fd: pidfd.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    // SAFETY: one valid pollfd.
                    while unsafe { libc::poll(&mut poll_fd, 1, -1) } < 0
                        && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
                    {
                    }
                    lifeline.cut();
                });
            }
        }
    }
}

/// Runs in the helper's fork, single-threaded: the supervisor. Never
/// returns. `start_bubblewrap` runs in the supervisor's child, gets the
/// supervisor's pid and must exec bubblewrap (the child exits 127 if it
/// returns). `close_after_fork` are
/// descriptors the supervisor itself must not keep. Signals the helper
/// forwards stay blocked here (they reach bubblewrap through the process
/// group), as does `SIGCHLD`: `start_bubblewrap` must set its signal mask.
pub(crate) fn run(
    lifeline: OwnedFd,
    close_after_fork: &[RawFd],
    start_bubblewrap: impl FnOnce(libc::pid_t),
) -> ! {
    // SAFETY: plain syscalls on this process.
    unsafe {
        if libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) < 0 {
            let err = std::io::Error::last_os_error();
            panic!("failed to make the sandbox supervisor a subreaper: {err}");
        }
        libc::signal(libc::SIGCHLD, libc::SIG_DFL);
    }
    let child_signals = SignalFd::sigchld();
    // SAFETY: plain syscall.
    let supervisor = unsafe { libc::getpid() };
    // SAFETY: the supervisor is single-threaded.
    let bubblewrap = unsafe { libc::fork() };
    if bubblewrap < 0 {
        let err = std::io::Error::last_os_error();
        panic!("failed to fork for bubblewrap: {err}");
    }
    if bubblewrap == 0 {
        start_bubblewrap(supervisor);
        // SAFETY: ends the child, which failed to exec bubblewrap.
        unsafe { libc::_exit(127) }
    }
    // What bubblewrap inherits (its arguments' descriptors) is not kept here.
    drop(start_bubblewrap);
    for fd in close_after_fork.iter().copied().filter(|fd| *fd >= 0) {
        // SAFETY: closes this process's copy of a descriptor it does not use.
        unsafe {
            libc::close(fd);
        }
    }

    let mut status = None;
    loop {
        let mut fds = [
            libc::pollfd {
                fd: lifeline.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: child_signals.0.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: two valid pollfds.
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
        if ready < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            break;
        }
        if fds[1].revents != 0 {
            child_signals.drain();
            reap_exited(bubblewrap, &mut status);
            if status.is_some() {
                break;
            }
        }
        if fds[0].revents != 0 {
            // The helper is gone or stopped the command.
            break;
        }
    }
    if status.is_none() {
        // SAFETY: bubblewrap is this process's unreaped child.
        unsafe {
            libc::kill(bubblewrap, libc::SIGKILL);
        }
    }
    kill_and_reap_children(bubblewrap, &mut status);
    exit_like(status.unwrap_or(libc::SIGKILL))
}

/// Reaps every exited child; records bubblewrap's status.
fn reap_exited(bubblewrap: libc::pid_t, status: &mut Option<libc::c_int>) {
    loop {
        let mut wait_status = 0;
        // SAFETY: valid out pointer.
        let pid = unsafe { libc::waitpid(-1, &mut wait_status, libc::WNOHANG | libc::__WALL) };
        if pid <= 0 {
            return;
        }
        if pid == bubblewrap {
            *status = Some(wait_status);
        }
    }
}

/// Kills and reaps every remaining child: bubblewrap if still running, and
/// any sandbox PID 1 it left behind. A child's pid cannot be reused before
/// it is reaped here, so the kill never reaches another process.
fn kill_and_reap_children(bubblewrap: libc::pid_t, status: &mut Option<libc::c_int>) {
    loop {
        let Some(children) = children() else {
            // Cannot list them (no /proc): reap what has exited rather than
            // wait for a child that may never exit.
            reap_exited(bubblewrap, status);
            return;
        };
        for child in children {
            // SAFETY: an unreaped child of this process.
            unsafe {
                libc::kill(child, libc::SIGKILL);
            }
        }
        let mut wait_status = 0;
        // SAFETY: valid out pointer.
        let pid = unsafe { libc::waitpid(-1, &mut wait_status, libc::__WALL) };
        if pid == bubblewrap {
            *status = Some(wait_status);
        } else if pid < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            // ECHILD: none left.
            return;
        }
    }
}

/// This single-threaded process's children, from
/// `/proc/<pid>/task/<pid>/children`, or by scanning `/proc` where the
/// kernel lacks that file.
fn children() -> Option<Vec<libc::pid_t>> {
    // SAFETY: plain syscall.
    let me = unsafe { libc::getpid() };
    if let Ok(list) = std::fs::read_to_string(format!("/proc/{me}/task/{me}/children")) {
        return Some(
            list.split_ascii_whitespace()
                .filter_map(|pid| pid.parse().ok())
                .collect(),
        );
    }
    let mut children = Vec::new();
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // `pid (comm) state ppid …`; `comm` may contain anything but the
        // last `)`.
        let parent = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_ascii_whitespace().nth(1))
            .and_then(|ppid| ppid.parse::<libc::pid_t>().ok());
        if parent == Some(me) {
            children.push(pid);
        }
    }
    Some(children)
}

/// Exits the way `status` says bubblewrap did.
fn exit_like(status: libc::c_int) -> ! {
    if libc::WIFEXITED(status) {
        // SAFETY: ends this forked process without running its parent's
        // exit handlers.
        unsafe { libc::_exit(libc::WEXITSTATUS(status)) }
    }
    let signal = if libc::WIFSIGNALED(status) {
        libc::WTERMSIG(status)
    } else {
        libc::SIGKILL
    };
    // SAFETY: plain syscalls on this process; the signal is delivered on
    // unblocking it. The helper re-raises it too; one core dump at most.
    unsafe {
        let no_core = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        libc::setrlimit(libc::RLIMIT_CORE, &no_core);
        libc::signal(signal, libc::SIG_DFL);
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, signal);
        libc::kill(libc::getpid(), signal);
        libc::sigprocmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
        libc::_exit(128 + signal)
    }
}

/// `SIGCHLD`, blocked and read through a signalfd.
struct SignalFd(OwnedFd);

impl SignalFd {
    fn sigchld() -> Self {
        // SAFETY: plain syscalls with valid sets.
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGCHLD);
            if libc::sigprocmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) < 0 {
                let err = std::io::Error::last_os_error();
                panic!("failed to block SIGCHLD in the sandbox supervisor: {err}");
            }
            let fd = libc::signalfd(-1, &set, libc::SFD_CLOEXEC | libc::SFD_NONBLOCK);
            if fd < 0 {
                let err = std::io::Error::last_os_error();
                panic!("failed to watch the sandbox supervisor's children: {err}");
            }
            Self(OwnedFd::from_raw_fd(fd))
        }
    }

    fn drain(&self) {
        let mut info: libc::signalfd_siginfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::signalfd_siginfo>();
        // SAFETY: reads at most `size` bytes into `info`.
        while unsafe {
            libc::read(
                self.0.as_raw_fd(),
                (&mut info as *mut libc::signalfd_siginfo).cast(),
                size,
            )
        } > 0
        {}
    }
}
