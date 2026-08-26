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
//! A row may carry the `content` it was read from — an article pasted in
//! whole. That is what makes its `chars` exact rather than pages × a constant,
//! and it feeds the same tokenization the live line stream does. The text itself is *not* loaded with the row: every day's
//! sessions are fetched to render a timeline, and an article body is orders of
//! magnitude larger than everything else on it. [`fetch_content`] reads one.

use jp_core::knowledge::Knowledge;
use sqlx::Row;

#[derive(Debug, serde::Serialize)]
pub struct ManualSession {
    pub id: i64,
    pub start_ts: f64,
    /// When the reading stopped, when it was actually timed. `None` for a
    /// session logged without minutes; the duration is derived instead. See
    /// `History::duration_of`.
    pub end_ts: Option<f64>,
    pub chars: i64,
    pub source: String,
    pub work: Option<String>,
    pub pages: Option<f64>,
    pub note: Option<String>,
    pub url: Option<String>,
    /// Whether `content` is present, without carrying it. See the module doc.
    pub has_content: bool,
}

/// The columns [`manual_session_from_row`] expects — `content` only as a flag.
const COLUMNS: &str = "id, start_ts, end_ts, chars, source, work, pages, note, url, \
                       content IS NOT NULL AS has_content";

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
        url: r.get("url"),
        has_content: r.get::<i64, _>("has_content") != 0,
    }
}

pub async fn fetch_sessions(
    k: &Knowledge,
    from_ts: f64,
    to_ts: f64,
) -> Result<Vec<ManualSession>, sqlx::Error> {
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM manual_sessions WHERE start_ts >= ? AND start_ts < ? ORDER BY start_ts",
    ))
    .bind(from_ts)
    .bind(to_ts)
    .fetch_all(k.pool())
    .await?;
    Ok(rows.iter().map(manual_session_from_row).collect())
}

/// A session to write. A struct rather than positional arguments because half
/// the columns are optional and adjacent ones share a type.
#[derive(Debug, Default)]
pub struct NewSession<'a> {
    pub start_ts: f64,
    pub end_ts: Option<f64>,
    pub chars: i64,
    pub source: &'a str,
    pub work: Option<&'a str>,
    pub pages: Option<f64>,
    pub note: Option<&'a str>,
    pub url: Option<&'a str>,
    pub content: Option<&'a str>,
}

pub async fn insert_session(
    k: &Knowledge,
    s: NewSession<'_>,
) -> Result<ManualSession, sqlx::Error> {
    let row = sqlx::query(&format!(
        "INSERT INTO manual_sessions (start_ts, end_ts, chars, source, work, pages, note, url, content) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING {COLUMNS}",
    ))
    .bind(s.start_ts)
    .bind(s.end_ts)
    .bind(s.chars)
    .bind(s.source)
    .bind(s.work)
    .bind(s.pages)
    .bind(s.note)
    .bind(s.url)
    .bind(s.content)
    .fetch_one(k.pool())
    .await?;
    Ok(manual_session_from_row(&row))
}

/// One session's text, for the tokenizer. `start_ts` rather than a per-line
/// timestamp because that is all there is: pasted text has no timing inside it.
pub struct SessionText {
    pub id: i64,
    pub start_ts: f64,
    pub content: String,
    pub source: String,
    pub work: Option<String>,
}

/// Sessions carrying text, past `after_id`. Drives the ingest watermark the
/// way `fetch_lines_after` drives the line one.
pub async fn fetch_session_texts_after(
    k: &Knowledge,
    after_id: i64,
) -> Result<Vec<SessionText>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, start_ts, content, source, work FROM manual_sessions \
         WHERE id > ? AND content IS NOT NULL ORDER BY id",
    )
    .bind(after_id)
    .fetch_all(k.pool())
    .await?;
    Ok(rows
        .iter()
        .map(|r| SessionText {
            id: r.get("id"),
            start_ts: r.get("start_ts"),
            content: r.get("content"),
            source: r.get("source"),
            work: r.get("work"),
        })
        .collect())
}

/// The text a session was logged from, if it carried one.
pub async fn fetch_content(k: &Knowledge, id: i64) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query("SELECT content FROM manual_sessions WHERE id = ?")
        .bind(id)
        .fetch_optional(k.pool())
        .await?;
    Ok(row.and_then(|r| r.get("content")))
}

pub async fn delete_session(k: &Knowledge, id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM manual_sessions WHERE id = ?")
        .bind(id)
        .execute(k.pool())
        .await?;
    Ok(result.rows_affected() > 0)
}
