//! `manual_sessions` — reading time entered by hand.
//!
//! Everything a texthooker can't see: physical books, ebooks, articles. The
//! row carries its own character count (exact, or pages × `chars_per_page`)
//! because there is no text to count, and it is *not* a derived session — the
//! per-day aggregates merge these in beside the derived ones rather than
//! reconciling the two.
//!
//! Named `manual_sessions` rather than `sessions` because a *derived* session
//! — a sitting cut out of the line stream — is the other meaning of that word
//! in this codebase, and it is computed, never stored.
//!
//! `spec/knowledge-db.md` gives it a `content` column next, so pasted text can
//! feed the same tokenization the live line stream does.

use jp_core::knowledge::Knowledge;
use sqlx::Row;

#[derive(Debug, serde::Serialize)]
pub struct ManualSession {
    pub id: i64,
    pub start_ts: f64,
    pub end_ts: f64,
    pub chars: i64,
    pub source: String,
    pub work: Option<String>,
    pub pages: Option<f64>,
    pub note: Option<String>,
}

fn manual_session_from_row(r: &sqlx::sqlite::SqliteRow) -> ManualSession {
    ManualSession {
        id: r.get("id"),
        start_ts: r.get("start_ts"),
        end_ts: r.get("end_ts"),
        chars: r.get("chars"),
        source: r.get("source"),
        work: r.get("work"),
        pages: r.get("pages"),
        note: r.get("note"),
    }
}

pub async fn fetch_sessions(
    k: &Knowledge,
    from_ts: f64,
    to_ts: f64,
) -> Result<Vec<ManualSession>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, start_ts, end_ts, chars, source, work, pages, note FROM manual_sessions WHERE start_ts >= ? AND start_ts < ? ORDER BY start_ts",
    )
    .bind(from_ts)
    .bind(to_ts)
    .fetch_all(k.pool())
    .await?;
    Ok(rows.iter().map(manual_session_from_row).collect())
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_session(
    k: &Knowledge,
    start_ts: f64,
    end_ts: f64,
    chars: i64,
    source: &str,
    work: Option<&str>,
    pages: Option<f64>,
    note: Option<&str>,
) -> Result<ManualSession, sqlx::Error> {
    let row = sqlx::query(
        "INSERT INTO manual_sessions (start_ts, end_ts, chars, source, work, pages, note) VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id, start_ts, end_ts, chars, source, work, pages, note",
    )
    .bind(start_ts)
    .bind(end_ts)
    .bind(chars)
    .bind(source)
    .bind(work)
    .bind(pages)
    .bind(note)
    .fetch_one(k.pool())
    .await?;
    Ok(manual_session_from_row(&row))
}

pub async fn delete_session(k: &Knowledge, id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM manual_sessions WHERE id = ?")
        .bind(id)
        .execute(k.pool())
        .await?;
    Ok(result.rows_affected() > 0)
}
