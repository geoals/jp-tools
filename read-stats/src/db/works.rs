//! `works` — the source dimension: one row per VN, book or article.
//!
//! The join key is the **exact title string** stamped onto `lines.work` and
//! `sessions.work`, not an id — vn-ws-logger.py writes the title it is given
//! and has no way to look up a foreign key. So a row here is metadata *about* a
//! title (cover, status, queue position, capture window), and a title with no
//! row still reads and counts perfectly well.

use sqlx::{Row, SqlitePool};

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

pub async fn fetch_works_meta(pool: &SqlitePool) -> Result<Vec<Work>, sqlx::Error> {
    let rows = sqlx::query(&format!("SELECT {WORK_COLS} FROM works ORDER BY id"))
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(work_from_row).collect())
}

pub async fn fetch_work(pool: &SqlitePool, id: i64) -> Result<Option<Work>, sqlx::Error> {
    let row = sqlx::query(&format!("SELECT {WORK_COLS} FROM works WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.as_ref().map(work_from_row))
}

/// Get-or-create a work row by its exact title (the lines/sessions join key).
pub async fn upsert_work(pool: &SqlitePool, title: &str) -> Result<Work, sqlx::Error> {
    sqlx::query("INSERT INTO works (title) VALUES (?) ON CONFLICT(title) DO NOTHING")
        .bind(title)
        .execute(pool)
        .await?;
    let row = sqlx::query(&format!("SELECT {WORK_COLS} FROM works WHERE title = ?"))
        .bind(title)
        .fetch_one(pool)
        .await?;
    Ok(work_from_row(&row))
}

pub async fn set_work_cover(
    pool: &SqlitePool,
    id: i64,
    cover_path: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE works SET cover_path = ? WHERE id = ?")
        .bind(cover_path)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_work_total_chars(
    pool: &SqlitePool,
    id: i64,
    total_chars: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE works SET total_chars = ? WHERE id = ?")
        .bind(total_chars)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_work_status(pool: &SqlitePool, id: i64, status: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE works SET status = ? WHERE id = ?")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_work_queue_pos(
    pool: &SqlitePool,
    id: i64,
    queue_pos: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE works SET queue_pos = ? WHERE id = ?")
        .bind(queue_pos)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_work_vn_window(
    pool: &SqlitePool,
    id: i64,
    vn_window: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE works SET vn_window = ? WHERE id = ?")
        .bind(vn_window)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// The `vn_window` of whichever work is currently selected, if it has one set.
/// The capture target follows the current VN without a separate global knob.
pub async fn current_work_vn_window(pool: &SqlitePool) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT w.vn_window FROM works w
           JOIN settings s ON s.key = 'current_work' AND s.value = w.title",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|r| r.get::<Option<String>, _>("vn_window")))
}

pub async fn delete_work(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM works WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
