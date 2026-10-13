//! #616: a panic on a runtime worker must not leave `exec` waiting forever.
//!
//! Core finishes a turn whose task panicked with an error, but a panic in any
//! other task the turn depends on can still leave it without an end. Exec
//! records every panic in the process; once one has happened and the turn then
//! makes no progress for [`PANIC_STALL_TIMEOUT`], exec stops the turn and exits
//! non-zero instead of hanging.

use std::sync::Once;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::Instant;

/// How long a turn may go without any event after a panic before exec gives
/// up on it. Long enough for a slow model response, short enough that a hung
/// automation run is noticed.
pub(crate) const PANIC_STALL_TIMEOUT: Duration = Duration::from_secs(300);

pub(crate) const PANIC_STALL_MESSAGE: &str = "an internal error (a panic) stopped this run from \
     making progress; exiting. The details are in the log.";

/// Panics observed in this process.
pub(crate) struct PanicSignal {
    count: AtomicU64,
    notify: Notify,
}

impl PanicSignal {
    pub(crate) const fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            notify: Notify::const_new(),
        }
    }

    pub(crate) fn record(&self) {
        self.count.fetch_add(1, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    fn panicked(&self) -> bool {
        self.count.load(Ordering::SeqCst) > 0
    }

    async fn wait_for_panic(&self) {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.panicked() {
                return;
            }
            notified.await;
        }
    }
}

pub(crate) static PROCESS_PANICS: PanicSignal = PanicSignal::new();

/// Records every panic in [`PROCESS_PANICS`], keeping the existing hook's
/// report. A panic inside a Vault scoped-credential callback is recovered
/// there and neither formatted nor recorded.
pub(crate) fn install_panic_hook() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if codex_vault::scoped_credential_callback_active() {
                return;
            }
            PROCESS_PANICS.record();
            previous(info);
        }));
    });
}

/// Tracks a turn's progress against a [`PanicSignal`].
pub(crate) struct PanicStallWatch {
    signal: &'static PanicSignal,
    timeout: Duration,
    last_progress: Instant,
}

impl PanicStallWatch {
    pub(crate) fn new(signal: &'static PanicSignal, timeout: Duration) -> Self {
        Self {
            signal,
            timeout,
            last_progress: Instant::now(),
        }
    }

    /// Notes that the turn made progress (an event arrived).
    pub(crate) fn progress(&mut self) {
        self.last_progress = Instant::now();
    }

    /// Completes once a panic has happened and the turn has then gone
    /// `timeout` without progress. Never completes without a panic.
    pub(crate) async fn stalled(&self) {
        self.signal.wait_for_panic().await;
        let noticed = Instant::now();
        tokio::time::sleep_until(self.last_progress.max(noticed) + self.timeout).await;
    }
}

#[cfg(test)]
#[path = "panic_watch_tests.rs"]
mod tests;
