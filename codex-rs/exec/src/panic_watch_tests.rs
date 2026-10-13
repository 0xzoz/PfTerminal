use super::*;
use pretty_assertions::assert_eq;

const TIMEOUT: Duration = Duration::from_secs(300);

#[tokio::test(start_paused = true)]
async fn a_stall_needs_a_panic() {
    static SIGNAL: PanicSignal = PanicSignal::new();
    let watch = PanicStallWatch::new(&SIGNAL, TIMEOUT);

    let waited = tokio::time::timeout(TIMEOUT * 10, watch.stalled()).await;

    assert!(waited.is_err(), "no panic happened, so no stall");
}

#[tokio::test(start_paused = true)]
async fn a_stall_after_a_panic_needs_a_full_timeout_without_progress() {
    static SIGNAL: PanicSignal = PanicSignal::new();
    let mut watch = PanicStallWatch::new(&SIGNAL, TIMEOUT);
    SIGNAL.record();
    tokio::time::advance(TIMEOUT / 2).await;
    watch.progress();

    let early = tokio::time::timeout(TIMEOUT - Duration::from_secs(1), watch.stalled()).await;
    assert!(early.is_err(), "progress restarted the stall timer");

    let started = Instant::now();
    watch.stalled().await;
    assert_eq!(started.elapsed(), TIMEOUT);
}

/// #616: a panicking Tokio task is seen by the hook exec installs, and the
/// stalled run then ends instead of waiting forever.
#[tokio::test(start_paused = true)]
async fn a_panicking_runtime_task_ends_a_stalled_run() {
    install_panic_hook();
    let watch = PanicStallWatch::new(&PROCESS_PANICS, TIMEOUT);

    let joined = tokio::spawn(async { panic!("#616 simulated worker panic") }).await;
    assert!(joined.expect_err("task panicked").is_panic());
    assert!(PROCESS_PANICS.panicked());

    tokio::time::timeout(TIMEOUT * 2, watch.stalled())
        .await
        .expect("a stalled run after a panic ends");
}
