//! #314 (D228): the UTC days on which accounting failed to record at least
//! one of a conversation's model requests.
//!
//! Such a request was sent and billed but has no record, so the day's totals
//! may be incomplete; `/cost` says so instead of letting the missing work read
//! as free. A marker names a conversation and a UTC day only. The table comes
//! with ledger format 7, which a ledger takes on only when its first marker is
//! written; every read here first checks that the table exists.
use anyhow::ensure;
use codex_protocol::ThreadId;
use sqlx::SqliteConnection;

const DAY_MS: i64 = 86_400_000;

/// The ledger format that holds gap markers.
pub(in crate::runtime) const FORMAT: i64 = 7;

/// Whether this ledger has the gap-marker table (format 7 or later).
pub(in crate::runtime) async fn installed(conn: &mut SqliteConnection) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = 'draft_accounting_gap_days' AND type = 'table')",
    )
    .fetch_one(conn)
    .await?)
}

/// Mark `owner`'s UTC day containing `at_ms`. The caller holds the write
/// transaction and has upgraded the ledger to `FORMAT`. Marking a day twice
/// is one marker.
pub(in crate::runtime) async fn mark_on_connection(
    conn: &mut SqliteConnection,
    owner: ThreadId,
    at_ms: i64,
) -> anyhow::Result<()> {
    ensure!(at_ms >= 0, "negative gap time");
    sqlx::query(
        "INSERT INTO draft_accounting_gap_days VALUES (?, ?) ON CONFLICT(thread_id, utc_day) DO NOTHING",
    )
    .bind(owner.to_string())
    .bind(at_ms.div_euclid(DAY_MS))
    .execute(conn)
    .await?;
    Ok(())
}

/// Remove a deleted conversation's markers with the rest of its accounting.
pub(in crate::runtime) async fn remove_owner(
    conn: &mut SqliteConnection,
    thread: &str,
) -> anyhow::Result<()> {
    if installed(conn).await? {
        sqlx::query("DELETE FROM draft_accounting_gap_days WHERE thread_id = ?")
            .bind(thread)
            .execute(conn)
            .await?;
    }
    Ok(())
}

/// Remove the markers of days whose totals retention has expired at
/// `as_of_ms`, as it expires a compact day: `utc_day * DAY + replay <= as_of`.
pub(in crate::runtime) async fn expire(
    conn: &mut SqliteConnection,
    as_of_ms: i64,
    replay_ms: i64,
) -> anyhow::Result<()> {
    if installed(conn).await? {
        sqlx::query("DELETE FROM draft_accounting_gap_days WHERE utc_day * ? + ? <= ?")
            .bind(DAY_MS)
            .bind(replay_ms)
            .bind(as_of_ms)
            .execute(conn)
            .await?;
    }
    Ok(())
}
