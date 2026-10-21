//! FUP-02 (#352): a past day with nothing recorded says "no requests" only
//! when it ends before the conversation started; every other empty day stays
//! unproven, and the rule never applies to range buckets.

use super::*;
use pretty_assertions::assert_eq;

const DAY: i64 = 86_400_000;
/// The default read: day 400, 00:00 UTC. Day 299 is then 101 days back,
/// past the 90-day request detail and inside a year of daily totals (kept
/// from day 36).
const NOW: i64 = 400 * DAY;

/// A UUIDv7 thread id whose embedded time is `ms`.
fn v7(ms: i64) -> ThreadId {
    let ms = u64::try_from(ms).unwrap();
    let uuid = Uuid::new_v7(uuid::Timestamp::from_unix(
        uuid::NoContext,
        ms / 1000,
        u32::try_from(ms % 1000).unwrap() * 1_000_000,
    ));
    ThreadId::from_string(&uuid.to_string()).unwrap()
}

/// A native thread row whose stored creation time is `created_ms`.
async fn thread(runtime: &StateRuntime, id: ThreadId, created_ms: i64) -> anyhow::Result<()> {
    let home = runtime.sqlite().home();
    let mut metadata = test_thread_metadata(home, id, home.to_path_buf());
    metadata.created_at = chrono::DateTime::from_timestamp_millis(created_ms).unwrap();
    runtime.upsert_thread(&metadata).await
}

async fn set(
    runtime: &StateRuntime,
    sql: &'static str,
    value: impl ToString,
) -> anyhow::Result<()> {
    sqlx::query(sql)
        .bind(value.to_string())
        .execute(runtime.pool.as_ref())
        .await?;
    Ok(())
}

/// The ledger's checkpoint, as maintenance or a write would leave it.
async fn checkpoint(runtime: &StateRuntime, ms: i64) -> anyhow::Result<()> {
    set(
        runtime,
        "UPDATE draft_accounting_retention_checkpoint SET completed_as_of_ms = CAST(? AS INTEGER), admission_active = 1",
        ms,
    )
    .await
}

/// When this home's ledger was installed, as SQLite stamped it.
async fn installed(runtime: &StateRuntime, day: i64) -> anyhow::Result<()> {
    let at = chrono::DateTime::from_timestamp_millis(day * DAY + 3_600_000).unwrap();
    set(
        runtime,
        "UPDATE _accounting_migrations SET installed_on = ? WHERE version = 1",
        at.format("%Y-%m-%d %H:%M:%S"),
    )
    .await
}

/// A ledger installed on day 0 and current to `now`, and a conversation whose
/// id and stored creation time are both `start_ms`.
async fn fixture(
    path: &Path,
    start_ms: i64,
    now: i64,
) -> anyhow::Result<(Arc<StateRuntime>, ThreadId)> {
    let runtime = open(path).await?;
    let owner = v7(start_ms);
    thread(&runtime, owner, start_ms).await?;
    AccountingStore::open(&runtime, /*as_of*/ 0).await?;
    installed(&runtime, /*day*/ 0).await?;
    checkpoint(&runtime, now).await?;
    Ok((runtime, owner))
}

async fn record(
    runtime: &StateRuntime,
    id: u128,
    thread: ThreadId,
    dispatched_ms: i64,
) -> anyhow::Result<()> {
    let mut a = attempt(id);
    a.thread_id = thread;
    a.dispatched_at_ms = dispatched_ms.try_into()?;
    AccountingStore::open(runtime, dispatched_ms)
        .await?
        .admit(thread, &a, &[], dispatched_ms)
        .await?;
    Ok(())
}

async fn empty(
    runtime: &StateRuntime,
    owner: ThreadId,
    day: i64,
    now: i64,
) -> anyhow::Result<Option<EmptyDay>> {
    let view = inspection(AccountingStore::inspect_day(runtime, owner, day, now).await?);
    assert_eq!(view.totals.attempts, 0, "day {day}");
    Ok(view.empty_day)
}

fn before(started_day: i64) -> Option<EmptyDay> {
    Some(EmptyDay::BeforeConversation { started_day })
}

#[tokio::test]
async fn a_day_before_the_conversation_started_has_no_requests() -> anyhow::Result<()> {
    let path = home();
    let (runtime, owner) = fixture(&path, 300 * DAY, NOW).await?;
    // #352 run Z: past the request detail, inside the daily totals. This read
    // used to say the day's detail had expired.
    assert_eq!(empty(&runtime, owner, 299, NOW).await?, before(300));
    // Even below the daily totals' floor: the proof needs no retained row.
    assert_eq!(empty(&runtime, owner, 0, NOW).await?, before(300));
    // From the day it started, nothing recorded proves nothing.
    for day in [300, 305, 350, 399, 400] {
        assert_eq!(
            empty(&runtime, owner, day, NOW).await?,
            Some(EmptyDay::NotRecorded),
            "day {day}"
        );
    }
    runtime.close().await;
    Ok(())
}

#[tokio::test]
async fn a_day_with_requests_is_not_an_empty_day() -> anyhow::Result<()> {
    let path = home();
    let (runtime, owner) = fixture(&path, 300 * DAY, 300 * DAY).await?;
    record(&runtime, 1, owner, 350 * DAY + 5).await?;
    checkpoint(&runtime, NOW).await?;
    let view = inspection(AccountingStore::inspect_day(&runtime, owner, 350, NOW).await?);
    assert_eq!((view.totals.attempts, view.empty_day), (1, None));
    runtime.close().await;
    Ok(())
}

#[tokio::test]
async fn the_start_is_compared_by_utc_day() -> anyhow::Result<()> {
    // 23:59:59.999 UTC on day 299: that day may hold requests, the one before
    // cannot. Its stored creation time is whole seconds, 23:59:59.
    let path = home();
    let (runtime, owner) = fixture(&path, 300 * DAY - 1, NOW).await?;
    assert_eq!(
        empty(&runtime, owner, 299, NOW).await?,
        Some(EmptyDay::NotRecorded)
    );
    assert_eq!(empty(&runtime, owner, 298, NOW).await?, before(299));
    runtime.close().await;
    // 00:00:00.000 UTC on day 300: day 299 ends before it.
    let path = home();
    let (runtime, owner) = fixture(&path, 300 * DAY, NOW).await?;
    assert_eq!(empty(&runtime, owner, 299, NOW).await?, before(300));
    assert_eq!(
        empty(&runtime, owner, 300, NOW).await?,
        Some(EmptyDay::NotRecorded)
    );
    runtime.close().await;
    Ok(())
}

#[tokio::test]
async fn past_the_daily_totals_only_the_proven_day_is_shown() -> anyhow::Result<()> {
    // Read on day 800: daily totals are kept from day 436.
    let now = 800 * DAY;
    let path = home();
    let (runtime, owner) = fixture(&path, 300 * DAY, now).await?;
    assert_eq!(empty(&runtime, owner, 299, now).await?, before(300));
    assert!(matches!(
        AccountingStore::inspect_day(&runtime, owner, 350, now).await?,
        InspectionDay::DetailUnavailable {
            compact: false,
            coverage: RetentionCoverage {
                aggregate_day_floor: 436,
                ..
            },
            ..
        }
    ));
    assert_eq!(
        empty(&runtime, owner, 436, now).await?,
        Some(EmptyDay::NotRecorded)
    );
    runtime.close().await;
    Ok(())
}

#[tokio::test]
async fn a_day_before_the_ledger_existed_is_not_recorded() -> anyhow::Result<()> {
    let path = home();
    let (runtime, owner) = fixture(&path, 300 * DAY, NOW).await?;
    installed(&runtime, /*day*/ 320).await?;
    assert_eq!(
        empty(&runtime, owner, 310, NOW).await?,
        Some(EmptyDay::BeforeLedger {
            ledger_started_day: 320
        })
    );
    assert_eq!(
        empty(&runtime, owner, 320, NOW).await?,
        Some(EmptyDay::NotRecorded)
    );
    // The conversation's start still proves the days before it.
    assert_eq!(empty(&runtime, owner, 299, NOW).await?, before(300));
    // An install time this build cannot read states nothing about the ledger.
    set(
        &runtime,
        "UPDATE _accounting_migrations SET installed_on = ? WHERE version = 1",
        "unreadable",
    )
    .await?;
    assert_eq!(
        empty(&runtime, owner, 310, NOW).await?,
        Some(EmptyDay::NotRecorded)
    );
    runtime.close().await;
    Ok(())
}

#[tokio::test]
async fn pending_or_lagging_accounting_is_never_an_empty_day() -> anyhow::Result<()> {
    let path = home();
    // The ledger last moved on day 250; the conversation started on day 300.
    let (runtime, owner) = fixture(&path, 300 * DAY, 250 * DAY).await?;
    // After the checkpoint nothing is verified yet, not even before the start.
    assert_eq!(
        AccountingStore::inspect_day(&runtime, owner, 260, 301 * DAY).await?,
        InspectionDay::CheckpointLag
    );
    assert_eq!(empty(&runtime, owner, 240, 301 * DAY).await?, before(300));
    runtime.close().await;

    // A request older than the request detail that maintenance has not
    // compacted yet is pending, not absent.
    let path = home();
    let (runtime, owner) = fixture(&path, 300 * DAY, 300 * DAY).await?;
    record(&runtime, 1, owner, 305 * DAY).await?;
    checkpoint(&runtime, NOW).await?;
    assert_eq!(
        AccountingStore::inspect_day(&runtime, owner, 305, NOW).await?,
        InspectionDay::CheckpointLag
    );
    runtime.close().await;
    Ok(())
}

#[tokio::test]
async fn other_threads_on_the_day_can_only_withdraw_the_proof() -> anyhow::Result<()> {
    // Read on day 320, so every request below keeps its detail.
    let now = 320 * DAY;
    let path = home();
    let (runtime, owner) = fixture(&path, 300 * DAY, 290 * DAY).await?;
    // A subagent whose request is recorded on day 295, before the root's id
    // time: the day is the conversation's, and the start moves back to it.
    let child = v7(310 * DAY);
    thread(&runtime, child, 310 * DAY).await?;
    runtime
        .upsert_thread_spawn_edge(
            owner,
            child,
            crate::DirectionalThreadSpawnEdgeStatus::Closed,
        )
        .await?;
    record(&runtime, 1, child, 295 * DAY).await?;
    checkpoint(&runtime, now).await?;
    let view = inspection(AccountingStore::inspect_day(&runtime, owner, 295, now).await?);
    assert_eq!((view.totals.attempts, view.empty_day), (1, None));
    assert_eq!(empty(&runtime, owner, 294, now).await?, before(295));
    // A thread of unknown ancestry with a request on day 293 may be an
    // unlinked subagent: that day is not proven.
    let stranger = v7(200 * DAY);
    let home = runtime.sqlite().home();
    let mut metadata = test_thread_metadata(home, stranger, home.to_path_buf());
    metadata.source = "unknown".to_string();
    runtime.upsert_thread(&metadata).await?;
    set(
        &runtime,
        "UPDATE draft_accounting_retention_checkpoint SET completed_as_of_ms = CAST(? AS INTEGER)",
        293 * DAY,
    )
    .await?;
    record(&runtime, 2, stranger, 293 * DAY).await?;
    checkpoint(&runtime, now).await?;
    let view = inspection(AccountingStore::inspect_day(&runtime, owner, 293, now).await?);
    assert_eq!(
        (view.unknown_parent_totals.attempts, view.empty_day),
        (1, Some(EmptyDay::NotRecorded))
    );
    assert_eq!(empty(&runtime, owner, 292, now).await?, before(295));
    runtime.close().await;
    Ok(())
}

#[tokio::test]
async fn the_start_is_the_earliest_evidence_and_needs_a_time_bearing_id() -> anyhow::Result<()> {
    // A stored creation time rewritten later (a placeholder upsert) cannot
    // move the start past the id's time.
    let path = home();
    let (runtime, owner) = fixture(&path, 300 * DAY, NOW).await?;
    thread(&runtime, owner, 390 * DAY).await?;
    assert_eq!(empty(&runtime, owner, 299, NOW).await?, before(300));
    // An earlier stored creation time moves it back.
    thread(&runtime, owner, 280 * DAY).await?;
    assert_eq!(
        empty(&runtime, owner, 299, NOW).await?,
        Some(EmptyDay::NotRecorded)
    );
    assert_eq!(empty(&runtime, owner, 279, NOW).await?, before(280));
    // A subagent id that carries no time withdraws the proof.
    let legacy = ThreadId::from_string(&Uuid::from_u128(77).to_string())?;
    runtime
        .upsert_thread_spawn_edge(
            owner,
            legacy,
            crate::DirectionalThreadSpawnEdgeStatus::Closed,
        )
        .await?;
    assert_eq!(
        empty(&runtime, owner, 279, NOW).await?,
        Some(EmptyDay::NotRecorded)
    );
    runtime.close().await;
    // So does the conversation's own.
    let path = home();
    let runtime = open(&path).await?;
    thread(&runtime, legacy, 300 * DAY).await?;
    AccountingStore::open(&runtime, /*as_of*/ 0).await?;
    installed(&runtime, /*day*/ 0).await?;
    checkpoint(&runtime, NOW).await?;
    assert_eq!(
        empty(&runtime, legacy, 100, NOW).await?,
        Some(EmptyDay::NotRecorded)
    );
    assert!(matches!(
        AccountingStore::inspect_day(&runtime, legacy, 0, NOW).await?,
        InspectionDay::DetailUnavailable { compact: false, .. }
    ));
    runtime.close().await;
    Ok(())
}

#[tokio::test]
async fn range_buckets_never_classify_an_empty_day() -> anyhow::Result<()> {
    let path = home();
    let (runtime, owner) = fixture(&path, 300 * DAY, NOW).await?;
    let range = |start: i64| {
        AccountingStore::inspect_range(
            &runtime,
            owner,
            InspectionRange {
                start_ms: start * DAY,
                end_ms: (start + 1) * DAY,
                grouping: InspectionGrouping::Day,
            },
            NOW,
        )
    };
    let InspectionDay::Range { buckets, .. } = range(350).await? else {
        panic!()
    };
    let [InspectionDay::Ready(view)] = buckets[0].days.as_slice() else {
        panic!("{buckets:?}")
    };
    assert_eq!(view.empty_day, None);
    // Past the request detail a range keeps today's rule: unavailable.
    let InspectionDay::Range { buckets, .. } = range(299).await? else {
        panic!()
    };
    assert!(matches!(
        buckets[0].days.as_slice(),
        [InspectionDay::DetailUnavailable { compact: false, .. }]
    ));
    runtime.close().await;
    Ok(())
}
