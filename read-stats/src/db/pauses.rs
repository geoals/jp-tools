//! `pauses` — spans of wall-clock time that don't count as reading.
//!
//! A row with `end_ts IS NULL` is an open pause: tracking is off right now.
//! The table is an interval log rather than a flag so that a pause taken hours
//! ago still removes exactly the lines it covered — see
//! [`crate::stats::is_paused`], which every read applies.

use sqlx::{Row, SqlitePool};

use crate::stats::PauseInterval;

pub async fn fetch_pauses(pool: &SqlitePool) -> Result<Vec<PauseInterval>, sqlx::Error> {
    let rows = sqlx::query("SELECT start_ts, end_ts FROM pauses ORDER BY start_ts")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| PauseInterval {
            start_ts: r.get("start_ts"),
            end_ts: r.get("end_ts"),
        })
        .collect())
}

/// Toggle the tracking pause; returns the new paused state.
pub async fn toggle_pause(pool: &SqlitePool, now: f64) -> Result<bool, sqlx::Error> {
    let open: Option<i64> = sqlx::query("SELECT id FROM pauses WHERE end_ts IS NULL LIMIT 1")
        .fetch_optional(pool)
        .await?
        .map(|r| r.get("id"));
    match open {
        Some(id) => {
            sqlx::query("UPDATE pauses SET end_ts = ? WHERE id = ?")
                .bind(now)
                .bind(id)
                .execute(pool)
                .await?;
            Ok(false)
        }
        None => {
            sqlx::query("INSERT INTO pauses (start_ts) VALUES (?)")
                .bind(now)
                .execute(pool)
                .await?;
            Ok(true)
        }
    }
}

pub async fn is_pause_open(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    Ok(
        sqlx::query("SELECT id FROM pauses WHERE end_ts IS NULL LIMIT 1")
            .fetch_optional(pool)
            .await?
            .is_some(),
    )
}
