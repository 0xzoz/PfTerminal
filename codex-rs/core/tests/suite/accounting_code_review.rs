//! #898: a `/review` code review is a sub-agent persisted as a thread of its
//! own, and that thread names no parent the ledger can resolve. Its paid
//! requests are recorded under the conversation that started the review,
//! labelled `code-review:`, so `/cost` counts them once in that conversation's
//! total instead of in an unresolved "unknown parent" population.
use super::accounting_responses::support::*;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ReviewRequest;
use codex_protocol::protocol::ReviewTarget;
use codex_state::accounting::*;
use core_test_support::responses;
use pretty_assertions::assert_eq;
use wiremock::MockServer;

/// The ledger is written by the collector, not the event stream: wait until
/// `count` attempts are recorded, and name what was found otherwise.
async fn wait_for_attempts(
    db: &codex_state::StateRuntime,
    count: usize,
) -> anyhow::Result<Vec<Attempt>> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let seen = attempts(db).await?;
        if seen.len() >= count {
            return Ok(seen);
        }
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!(
                "ledger holds {:?}",
                seen.iter().map(|record| &record.turn).collect::<Vec<_>>()
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accounting_code_review_is_recorded_under_the_conversation() -> anyhow::Result<()> {
    core_test_support::skip_if_no_network!(Ok(()));
    let server = MockServer::start().await;
    let endpoint = format!("{}/v1", server.uri());
    let review = serde_json::json!({
        "findings": [],
        "overall_correctness": "good",
        "overall_explanation": "fixture review",
        "overall_confidence_score": 0.5
    })
    .to_string();
    responses::mount_sse_sequence(
        &server,
        vec![
            success(usage(Some(0))),
            responses::sse(vec![
                responses::ev_response_created("review-response"),
                responses::ev_assistant_message("review-message", &review),
                event("response.completed", usage(Some(0))),
            ]),
        ],
    )
    .await;
    let test = builder(endpoint.clone(), enabled(&endpoint))
        .build_with_auto_env(&server)
        .await?;
    test.submit_turn("main conversation").await?;
    let db = test.codex.state_db().unwrap();
    wait_for_attempts(&db, /*count*/ 1).await?;

    test.codex
        .submit(Op::Review {
            review_request: ReviewRequest {
                target: ReviewTarget::Custom {
                    instructions: "Review the fixture".to_string(),
                },
                user_facing_hint: None,
            },
        })
        .await?;
    core_test_support::wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::ExitedReviewMode(_))
    })
    .await;
    let records = wait_for_attempts(&db, /*count*/ 2).await?;
    wait_observations(&db, /*count*/ 2).await?;
    // Nothing else is sent: no later attempt is in flight.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(attempts(&db).await?.len(), 2);
    assert_eq!(
        server.received_requests().await.unwrap_or_default().len(),
        2
    );

    let parent = test.session_configured.thread_id;
    let mut owners: Vec<_> = records
        .iter()
        .map(|attempt| {
            let kind = attempt
                .turn
                .rsplit_once(':')
                .map(|(kind, _)| format!("{kind}:"))
                .unwrap_or_default();
            (attempt.thread_id, kind)
        })
        .collect();
    owners.sort_by(|a, b| a.1.cmp(&b.1));
    assert_eq!(
        owners,
        vec![
            (parent, String::new()),
            (parent, "code-review:".to_string())
        ],
        "{records:#?}"
    );

    // `/cost`'s day view of the conversation counts the review once, in its
    // total; nothing is left in the unresolved population or elsewhere.
    let now = chrono::Utc::now().timestamp_millis();
    let day = i64::from(records[0].dispatched_at_ms) / 86_400_000;
    let InspectionDay::Ready(view) = AccountingStore::inspect_day(&db, parent, day, now).await?
    else {
        anyhow::bail!("the conversation's day is not readable");
    };
    assert_eq!(view.totals.attempts, 2);
    assert_eq!(view.requests.len(), 2);
    assert_eq!(view.unknown_parent_totals.attempts, 0);
    assert_eq!(
        view.other_conversations
            .as_ref()
            .map(|others| others.requests.len()),
        Some(0)
    );
    assert_eq!(
        view.requests
            .values()
            .flatten()
            .filter(|quote| quote.attempt.turn.starts_with("code-review:"))
            .count(),
        1
    );
    test.thread_manager
        .shutdown_all_threads_bounded(std::time::Duration::from_secs(3))
        .await;
    Ok(())
}
