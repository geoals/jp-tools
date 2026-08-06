//! `reader_marks` — deliberate actions taken on the #read page.
//!
//! Presence evidence only. Tapping explain, mine or clear proves the reader was
//! at the keyboard at that instant — and so does a retracted lookup, which
//! leaves one of these behind in place of the row it deleted
//! ([`crate::db::retract_lookup`]) — which is exactly what
//! [`crate::stats::Presence`] needs to credit the surrounding gap — but unlike
//! a lookup it says nothing about a word, so these rows are kept out of every
//! word metric on purpose and can't inflate the lookup count.

use sqlx::{Row, SqlitePool};

/// Record a presence mark from a #read action (explain / mine / clear / a
/// judgement that retracted its lookup). Best-
/// effort: the caller logs and swallows any error, since a missed mark only
/// costs a little presence credit and must never fail the action itself.
pub async fn insert_reader_mark(pool: &SqlitePool, ts: f64, kind: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO reader_marks (ts, kind) VALUES (?, ?)")
        .bind(ts)
        .bind(kind)
        .execute(pool)
        .await?;
    Ok(())
}

/// Reader-mark timestamps in a window, oldest first. Merged with lookups and
/// cards into the presence evidence stream.
pub async fn fetch_reader_marks(
    pool: &SqlitePool,
    from_ts: f64,
    to_ts: f64,
) -> Result<Vec<f64>, sqlx::Error> {
    let rows = sqlx::query("SELECT ts FROM reader_marks WHERE ts >= ? AND ts < ? ORDER BY ts")
        .bind(from_ts)
        .bind(to_ts)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| r.get("ts")).collect())
}
