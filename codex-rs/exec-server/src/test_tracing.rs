//! Test-only helpers for asserting on spans from a thread-scoped subscriber.

use tracing::Dispatch;
use tracing::Subscriber;
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::NoSubscriber;

/// Thread-scoped test subscriber whose spans cannot be disabled by concurrent tests.
///
/// tracing-core caches every callsite's interest process-wide. While exactly one
/// dispatcher is registered, it computes that interest from the default
/// subscriber of whichever thread hits a callsite first (or rebuilds the
/// cache). A concurrent test thread without a subscriber can then cache
/// `Interest::never` for a shared callsite such as `codex.exec_server.request`,
/// silently disabling that span for this test (#1413). Registering a second,
/// inert dispatcher for the guard's lifetime makes tracing-core consult every
/// live dispatcher, including this one, whenever it computes interest.
pub(crate) struct ScopedTestSubscriber {
    _default: DefaultGuard,
    _second_dispatcher: Dispatch,
}

impl ScopedTestSubscriber {
    pub(crate) fn set_default<S>(subscriber: S) -> Self
    where
        S: Subscriber + Send + Sync + 'static,
    {
        let second_dispatcher = Dispatch::new(NoSubscriber::default());
        let default = tracing::subscriber::set_default(subscriber);
        tracing::callsite::rebuild_interest_cache();
        Self {
            _default: default,
            _second_dispatcher: second_dispatcher,
        }
    }
}

mod tests {
    use super::ScopedTestSubscriber;

    fn shared_span_enabled() -> bool {
        !tracing::info_span!("scoped-test-subscriber-shared").is_disabled()
    }

    #[test]
    fn span_stays_enabled_when_another_thread_registers_callsite_first() {
        let _subscriber = ScopedTestSubscriber::set_default(tracing_subscriber::registry());

        // Like a concurrent test without a subscriber, this thread hits the
        // callsite first and so registers it.
        std::thread::spawn(shared_span_enabled)
            .join()
            .expect("other thread should finish");

        assert!(shared_span_enabled());
    }
}
