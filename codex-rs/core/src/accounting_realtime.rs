//! A realtime WebSocket session recorded as one request (#824).
//!
//! The session is admitted as one attempt when it opens, so it is in the
//! ledger however it ends, and its end time is recorded when it closes. Its
//! tokens are never reported to this client - the realtime events it parses
//! carry none - so they stay unknown, never zero.
use super::Sampling;
use super::responses::Slot;
use codex_login::CodexAuth;
use codex_model_provider_info::ModelProviderInfo;
use codex_state::accounting::Attempt;
use codex_state::accounting::Patch;
use std::sync::Arc;
use uuid::Uuid;

pub(crate) struct RealtimeSessionRecord {
    sampling: Arc<Sampling>,
    attempt: Attempt,
    source: Uuid,
}

impl RealtimeSessionRecord {
    /// Admit the session that opened at `url` (`route` under the approved
    /// endpoint) on `slot`'s collector. `None` when it goes unrecorded: no
    /// collector, a route the accounting does not approve, or a ledger
    /// failure. Never refuses the session.
    pub(crate) async fn begin(
        slot: &Slot,
        provider: &ModelProviderInfo,
        auth: Option<&CodexAuth>,
        url: &str,
        route: &str,
        model: &str,
    ) -> Option<Self> {
        let deferred = super::responses::read(slot).ok().flatten()?;
        let sampling = deferred
            .resolve_path(provider, auth, url, route)
            .await
            .ok()
            .flatten()?;
        match sampling.admit_with_tier(model, url, /*tier*/ None).await {
            Ok(attempt) => Some(Self {
                sampling,
                attempt,
                source: Uuid::new_v4(),
            }),
            Err(error) => {
                sampling.reject();
                super::gap("admit realtime session", format_args!("{error:#}"));
                None
            }
        }
    }

    /// Record when the session ended. Its tokens stay unknown.
    pub(crate) async fn end(self) {
        let ended = match super::now().try_into() {
            Ok(ended) => ended,
            Err(error) => {
                super::gap("record realtime session end", error);
                return;
            }
        };
        let patch = Patch {
            session_ended_at_ms: Some(ended),
            ..Patch::default()
        };
        if let Err(error) = self
            .sampling
            .observe_patch(&self.attempt, self.source, /*position*/ 1, patch)
            .await
        {
            self.sampling.reject();
            super::gap("record realtime session end", format_args!("{error:#}"));
        }
    }
}
