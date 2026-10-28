//! Realtime sessions on standalone WebSocket or existing WebRTC attempts.
//!
//! A WebSocket session is admitted when it opens; WebRTC retains the attempt
//! already admitted for HTTP creation. A WebSocket session's attempt then
//! records the token counts each finished response reported (#889 D223), as
//! running sums, and finally the session's end, which states only its
//! timestamp. No rate prices a session, so its cost stays unknown, never zero.
use super::Sampling;
use super::responses::DeferredResponsesSampling;
use codex_model_provider_info::ModelProviderInfo;
use codex_protocol::auth::AuthMode;
use codex_protocol::protocol::RealtimeResponseUsage;
use codex_state::accounting::Attempt;
use codex_state::accounting::Count;
use codex_state::accounting::Patch;
use codex_state::accounting::Presence;
use std::sync::Arc;
use uuid::Uuid;

pub(crate) struct RealtimeSessionRecord {
    sampling: Arc<Sampling>,
    attempt: Attempt,
    source: Uuid,
    /// The next observation's position: each usage report, then the end.
    next: i64,
    usage: SessionUsage,
    /// A usage write failed: nothing more is recorded for this session.
    stopped: bool,
}

impl RealtimeSessionRecord {
    /// Admit the session that opened at `url` (`route` under the approved
    /// endpoint) on `deferred`, with resolved `auth_mode`. `None` when it goes
    /// unrecorded: a route or credential the accounting does not approve, or
    /// a ledger failure, each logged as a gap. Never refuses the session.
    ///
    /// The attempt binds no rate (#889 D223): its tokens are audio and text
    /// at realtime rates no catalogue row states, so even a configured model
    /// that has a catalogue row leaves the session's cost unknown.
    pub(crate) async fn begin(
        deferred: &DeferredResponsesSampling,
        provider: &ModelProviderInfo,
        auth_mode: Option<AuthMode>,
        url: &str,
        route: &str,
        model: &str,
    ) -> Option<Self> {
        let sampling = deferred
            .resolve_path(provider, auth_mode, url, route)
            .await
            .ok()
            .flatten()?;
        match sampling.admit_without_rates(model, url).await {
            Ok(attempt) => Some(Self::for_attempt(sampling, attempt)),
            Err(error) => {
                sampling.reject();
                super::gap("admit realtime session", format_args!("{error:#}"));
                None
            }
        }
    }

    /// Observe a session on an already admitted HTTP call, without counting
    /// another request. A separate source keeps its observations distinct
    /// from any made while creating that call.
    pub(super) fn for_attempt(sampling: Arc<Sampling>, attempt: Attempt) -> Self {
        Self {
            sampling,
            attempt,
            source: Uuid::new_v4(),
            next: 1,
            usage: SessionUsage::default(),
            stopped: false,
        }
    }

    /// Record the tokens one finished response reported (#889 D223): the
    /// session's running sums, as one cumulative observation. A report the
    /// sums cannot take is left out (see `SessionUsage::add`); a write the
    /// ledger refuses ends recording for the session, logged as a gap.
    pub(crate) async fn observe_response(&mut self, usage: &RealtimeResponseUsage) {
        if self.stopped {
            return;
        }
        let Some(patch) = self.usage.add(usage) else {
            return;
        };
        let position = self.next;
        self.next += 1;
        if let Err(error) = self
            .sampling
            .observe_patch(&self.attempt, self.source, position, patch)
            .await
        {
            self.stopped = true;
            self.sampling.reject();
            super::gap("record realtime session usage", format_args!("{error:#}"));
        }
    }

    /// Record that the session ended at `ended_at_ms` (Unix ms), after every
    /// usage report. The write waits only while the state DB lock is held by
    /// another connection; a busy DB defers it and the close goes ahead.
    pub(crate) async fn end(self, ended_at_ms: i64) {
        let ended = match ended_at_ms.try_into() {
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
            .observe_patch(&self.attempt, self.source, self.next, patch)
            .await
        {
            self.sampling.reject();
            super::gap("record realtime session end", format_args!("{error:#}"));
        }
    }
}

/// Running sums of the token counts a session's responses reported
/// (#889 D223), in the order input, cached input, output, total.
///
/// The first response that reports any count fixes which counts the session
/// states. A later response that leaves one of them out, or whose cached
/// input exceeds its input, is not counted, so every recorded sum covers the
/// same responses. A total that disagrees with the response's own input plus
/// output is read as not stated.
#[derive(Debug, Default)]
struct SessionUsage {
    stated: Option<[bool; 4]>,
    sums: [i64; 4],
}

impl SessionUsage {
    /// Add one response's report; the cumulative patch to record, or `None`
    /// when this report changes nothing that is recorded.
    fn add(&mut self, usage: &RealtimeResponseUsage) -> Option<Patch> {
        let as_count = |value: Option<u64>| value.map(i64::try_from).transpose().ok();
        let mut counts = [
            as_count(usage.input_tokens)?,
            as_count(usage.cached_input_tokens)?,
            as_count(usage.output_tokens)?,
            as_count(usage.total_tokens)?,
        ];
        let [input, cached, output, total] = counts;
        if cached
            .zip(input)
            .is_some_and(|(cached, input)| cached > input)
        {
            tracing::debug!(
                target: "codex_core::accounting",
                "realtime response reported more cached input than input; not counted"
            );
            return None;
        }
        if let (Some(total), Some(input), Some(output)) = (total, input, output)
            && input.checked_add(output) != Some(total)
        {
            counts[3] = None;
        }
        let stated = *self
            .stated
            .get_or_insert(counts.map(|count| count.is_some()));
        let mut sums = self.sums;
        for ((sum, count), stated) in sums.iter_mut().zip(counts).zip(stated) {
            if !stated {
                continue;
            }
            let Some(count) = count else {
                tracing::debug!(
                    target: "codex_core::accounting",
                    "realtime response left out a count earlier responses stated; not counted"
                );
                return None;
            };
            *sum = sum.checked_add(count)?;
        }
        if !stated.contains(&true) {
            // Nothing stated yet: let a later report fix the shape.
            self.stated = None;
            return None;
        }
        self.sums = sums;
        let presence = |index: usize| {
            if stated[index] {
                Count::try_from(sums[index]).map_or(Presence::Missing, Presence::Number)
            } else {
                Presence::Missing
            }
        };
        Some(Patch {
            input: presence(0),
            read: presence(1),
            output: presence(2),
            total: presence(3),
            ..Patch::default()
        })
    }
}

#[cfg(test)]
#[path = "accounting_realtime_tests.rs"]
mod tests;
