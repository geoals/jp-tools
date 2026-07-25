//! One-time retirement of the `pauses` interval log.
//!
//! Pausing used to be retroactive: lines kept arriving during a pause and every
//! derivation filtered them out on read. That was the wrong half of the problem
//! — the raw stream still filled with text the reader had explicitly said was
//! not reading, and every query paid to remove it again. Capture now stops at
//! the source instead (`settings.capture_paused`, which vn-ws-logger.py polls),
//! so there is nothing to filter.
//!
//! What the old intervals covered still has to stop counting, though, or
//! dropping the filter would silently *add* that reading back to the history.
//! So before the table goes:
//!
//! - lines inside a pause get `discarded = 1` — the same retroactive flag the
//!   reader's "clear last" uses, which keeps the raw row and stays reversible;
//! - lookups and reader marks inside a pause are deleted, because neither has a
//!   discard flag and inventing one for a handful of rows would put a
//!   read-stats policy column on a shared knowledge table.
//!
//! Idempotent by construction: the table is dropped at the end, and everything
//! here is a no-op once it is gone.

use jp_core::knowledge::Knowledge;
use sqlx::{Row, SqlitePool};
use tracing::info;

/// `(start_ts, end_ts)`; an open interval (`end_ts IS NULL`) runs to now.
type Interval = (f64, Option<f64>);

pub async fn retire(local: &SqlitePool, knowledge: &Knowledge) -> Result<(), sqlx::Error> {
    let Some(intervals) = fetch(local).await? else {
        return Ok(());
    };
    if intervals.is_empty() {
        sqlx::query("DROP TABLE IF EXISTS pauses")
            .execute(local)
            .await?;
        return Ok(());
    }

    let mut lines = 0;
    let mut lookups = 0;
    let mut marks = 0;
    for (start, end) in &intervals {
        // An open interval ends at "now": it was never closed, so everything
        // after it was captured while paused.
        let end = end.unwrap_or(f64::MAX);
        lines += sqlx::query("UPDATE lines SET discarded = 1 WHERE ts >= ? AND ts < ?")
            .bind(start)
            .bind(end)
            .execute(knowledge.pool())
            .await?
            .rows_affected();
        lookups += sqlx::query("DELETE FROM lookups WHERE ts >= ? AND ts < ?")
            .bind(start)
            .bind(end)
            .execute(knowledge.pool())
            .await?
            .rows_affected();
        marks += sqlx::query("DELETE FROM reader_marks WHERE ts >= ? AND ts < ?")
            .bind(start)
            .bind(end)
            .execute(local)
            .await?
            .rows_affected();
    }

    sqlx::query("DROP TABLE IF EXISTS pauses")
        .execute(local)
        .await?;
    info!(
        intervals = intervals.len(),
        lines, lookups, marks, "retired the pauses table"
    );
    Ok(())
}

/// The intervals, or `None` when the table is already gone.
async fn fetch(pool: &SqlitePool) -> Result<Option<Vec<Interval>>, sqlx::Error> {
    let rows = sqlx::query("SELECT start_ts, end_ts FROM pauses ORDER BY start_ts")
        .fetch_all(pool)
        .await;
    match rows {
        Ok(rows) => Ok(Some(
            rows.iter()
                .map(|r| (r.get("start_ts"), r.get("end_ts")))
                .collect(),
        )),
        // No such table: already retired, or a fresh install.
        Err(sqlx::Error::Database(_)) => Ok(None),
        Err(e) => Err(e),
    }
}
