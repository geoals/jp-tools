//! `works` — the source dimension: one row per VN, book or article.
//!
//! The join key is the **exact title string** stamped onto `lines.work` and
//! `manual_sessions.work`, not an id — vn-ws-logger.py writes the title it is given
//! and has no way to look up a foreign key. So a row here is metadata *about* a
//! title (cover, status, queue position, capture window), and a title with no
//! row still reads and counts perfectly well.

use jp_core::knowledge::Knowledge;
use sqlx::Row;

pub const WORK_STATUSES: &[&str] = &["reading", "queued", "finished", "dropped"];

#[derive(Debug, serde::Serialize)]
pub struct Work {
    pub id: i64,
    pub title: String,
    pub total_chars: Option<i64>,
    pub cover_path: Option<String>,
    pub status: String,
    pub queue_pos: Option<i64>,
    /// Substring of this VN's window title, passed to vn-capture.sh as
    /// VN_WINDOW so a mine screenshots the VN rather than the focused window.
    /// Per-work so switching VNs switches the capture target with it.
    pub vn_window: Option<String>,
}

const WORK_COLS: &str = "id, title, total_chars, cover_path, status, queue_pos, vn_window";

fn work_from_row(r: &sqlx::sqlite::SqliteRow) -> Work {
    Work {
        id: r.get("id"),
        title: r.get("title"),
        total_chars: r.get("total_chars"),
        cover_path: r.get("cover_path"),
        status: r.get("status"),
        queue_pos: r.get("queue_pos"),
        vn_window: r.get("vn_window"),
    }
}

pub async fn fetch_works_meta(k: &Knowledge) -> Result<Vec<Work>, sqlx::Error> {
    let rows = sqlx::query(&format!("SELECT {WORK_COLS} FROM works ORDER BY id"))
        .fetch_all(k.pool())
        .await?;
    Ok(rows.iter().map(work_from_row).collect())
}

pub async fn fetch_work(k: &Knowledge, id: i64) -> Result<Option<Work>, sqlx::Error> {
    let row = sqlx::query(&format!("SELECT {WORK_COLS} FROM works WHERE id = ?"))
        .bind(id)
        .fetch_optional(k.pool())
        .await?;
    Ok(row.as_ref().map(work_from_row))
}

/// Get-or-create a work row by its exact title (the lines/sessions join key).
pub async fn upsert_work(k: &Knowledge, title: &str) -> Result<Work, sqlx::Error> {
    sqlx::query("INSERT INTO works (title) VALUES (?) ON CONFLICT(title) DO NOTHING")
        .bind(title)
        .execute(k.pool())
        .await?;
    let row = sqlx::query(&format!("SELECT {WORK_COLS} FROM works WHERE title = ?"))
        .bind(title)
        .fetch_one(k.pool())
        .await?;
    Ok(work_from_row(&row))
}

pub async fn set_work_cover(
    k: &Knowledge,
    id: i64,
    cover_path: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE works SET cover_path = ? WHERE id = ?")
        .bind(cover_path)
        .bind(id)
        .execute(k.pool())
        .await?;
    Ok(())
}

pub async fn set_work_total_chars(
    k: &Knowledge,
    id: i64,
    total_chars: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE works SET total_chars = ? WHERE id = ?")
        .bind(total_chars)
        .bind(id)
        .execute(k.pool())
        .await?;
    Ok(())
}

pub async fn set_work_status(k: &Knowledge, id: i64, status: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE works SET status = ? WHERE id = ?")
        .bind(status)
        .bind(id)
        .execute(k.pool())
        .await?;
    Ok(())
}

pub async fn set_work_queue_pos(
    k: &Knowledge,
    id: i64,
    queue_pos: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE works SET queue_pos = ? WHERE id = ?")
        .bind(queue_pos)
        .bind(id)
        .execute(k.pool())
        .await?;
    Ok(())
}

pub async fn set_work_vn_window(
    k: &Knowledge,
    id: i64,
    vn_window: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE works SET vn_window = ? WHERE id = ?")
        .bind(vn_window)
        .bind(id)
        .execute(k.pool())
        .await?;
    Ok(())
}

/// The `vn_window` of whichever work is currently selected, if it has one set.
/// The capture target follows the current VN without a separate global knob.
///
/// Two queries rather than a join: `works` lives in the shared knowledge
/// database and `settings` in read-stats' own, and one lookup by title is not
/// worth an ATTACH.
pub async fn current_work_vn_window(
    k: &Knowledge,
    current_work: &str,
) -> Result<Option<String>, sqlx::Error> {
    if current_work.is_empty() {
        return Ok(None);
    }
    let row = sqlx::query("SELECT vn_window FROM works WHERE title = ?")
        .bind(current_work)
        .fetch_optional(k.pool())
        .await?;
    Ok(row.and_then(|r| r.get::<Option<String>, _>("vn_window")))
}

pub async fn delete_work(k: &Knowledge, id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM works WHERE id = ?")
        .bind(id)
        .execute(k.pool())
        .await?;
    Ok(result.rows_affected() > 0)
}
