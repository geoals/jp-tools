//! `lines` — the raw hooked line stream, and the four shapes it is read in.
//!
//! vn-ws-logger.py appends here as Textractor hooks each text box; read-stats
//! only ever reads and flags. Nothing is deleted: a line that shouldn't count
//! gets `discarded = 1` and every read filters it out.
//!
//! The four shapes exist because the callers genuinely need different columns:
//!
//! - [`ReaderLine`] — id + text, for the reading view's live feed.
//! - [`WorkedLine`] / [`crate::stats::LineEvent`] — time + chars, for the
//!   derivations.
//! - [`crate::stats::WorkLine`] — time + chars + work, for per-VN totals.
//! - [`IngestLine`] — id + text, for tokenizing into `word_days`.
//!
//! Fetching all columns for all of them would be simpler and would also mean
//! dragging every line's text through the per-day aggregates.

use jp_core::knowledge::Knowledge;
use sqlx::Row;

use crate::stats::LineEvent;

/// A hooked line as the reader view shows it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReaderLine {
    pub id: i64,
    pub ts: f64,
    pub chars: i64,
    pub text: String,
}

/// Lines newer than `after_id`, oldest first. The reader's SSE loop calls this
/// on a short interval, so it stays a bounded index range scan.
pub async fn fetch_lines_after_id(
    k: &Knowledge,
    after_id: i64,
    limit: i64,
) -> Result<Vec<ReaderLine>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, ts, chars, text FROM lines
         WHERE id > ? AND text IS NOT NULL AND discarded = 0 ORDER BY id LIMIT ?",
    )
    .bind(after_id)
    .bind(limit)
    .fetch_all(k.pool())
    .await?;
    Ok(rows.iter().map(reader_line).collect())
}

/// The newest `limit` lines, oldest first — the backlog a reader gets on open
/// so the screen isn't blank until the next line is hooked.
pub async fn fetch_recent_lines(k: &Knowledge, limit: i64) -> Result<Vec<ReaderLine>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, ts, chars, text FROM lines
         WHERE text IS NOT NULL AND discarded = 0 ORDER BY id DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(k.pool())
    .await?;
    let mut lines: Vec<ReaderLine> = rows.iter().map(reader_line).collect();
    lines.reverse();
    Ok(lines)
}

/// Flag lines as not-reading (`discarded = 1`) or put them back. Every read of
/// the stream filters the flag out, so this is how a line stops counting
/// without leaving the raw table — the same reason pauses don't delete either.
///
/// Ids come from the client rather than being a "last N" computed here: the
/// reader is clearing the lines it has on screen, and a line hooked between
/// the tap and the request must not be swept up with them.
///
/// Returns the ids actually changed, which is what the undo button re-sends.
pub async fn set_lines_discarded(
    k: &Knowledge,
    ids: &[i64],
    discarded: bool,
) -> Result<Vec<i64>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "UPDATE lines SET discarded = ? WHERE id IN ({placeholders}) AND discarded = ?
         RETURNING id"
    );
    let mut q = sqlx::query(&sql).bind(i64::from(discarded));
    for id in ids {
        q = q.bind(id);
    }
    let rows = q.bind(i64::from(!discarded)).fetch_all(k.pool()).await?;
    Ok(rows.iter().map(|r| r.get("id")).collect())
}

fn reader_line(r: &sqlx::sqlite::SqliteRow) -> ReaderLine {
    ReaderLine {
        id: r.get("id"),
        ts: r.get("ts"),
        chars: r.get("chars"),
        text: r.get::<Option<String>, _>("text").unwrap_or_default(),
    }
}

/// Highest line id currently stored, or 0 when the table is empty.
pub async fn max_line_id(k: &Knowledge) -> Result<i64, sqlx::Error> {
    let row = sqlx::query("SELECT COALESCE(MAX(id), 0) AS max_id FROM lines")
        .fetch_one(k.pool())
        .await?;
    Ok(row.get("max_id"))
}

/// Every line of the sitting still in progress, oldest first — what a reader
/// opening the page gets, so the view starts with the whole session in front of
/// them rather than a fixed tail of it.
///
/// The boundary is the same one `stats::derive_sessions` splits on: walking
/// back from the newest line, the first gap over `session_gap_secs` ends it.
/// Derived here rather than read from a sessions table because there is no such
/// table — a session is a shape the line stream is read in, never a stored row.
///
/// `max` bounds it regardless, so a marathon sitting cannot hand a browser an
/// unbounded first paint; the reader scrolls back for anything past it exactly
/// as they would for an earlier session.
pub async fn fetch_current_session_lines(
    k: &Knowledge,
    session_gap_secs: f64,
    max: i64,
) -> Result<Vec<ReaderLine>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, ts, chars, text FROM lines
         WHERE text IS NOT NULL AND discarded = 0 ORDER BY id DESC LIMIT ?",
    )
    .bind(max)
    .fetch_all(k.pool())
    .await?;
    // Newest first here, so a gap is measured against the line *after* it in
    // reading order — the one already kept.
    let mut lines: Vec<ReaderLine> = Vec::new();
    for line in rows.iter().map(reader_line) {
        if let Some(last) = lines.last()
            && last.ts - line.ts > session_gap_secs
        {
            break;
        }
        lines.push(line);
    }
    lines.reverse();
    Ok(lines)
}

/// The `limit` lines immediately before `before_id`, oldest first — one page of
/// backscroll for the reader, which starts from the oldest id it currently
/// holds and asks for what came before it.
pub async fn fetch_lines_before_id(
    k: &Knowledge,
    before_id: i64,
    limit: i64,
) -> Result<Vec<ReaderLine>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, ts, chars, text FROM lines
         WHERE id < ? AND text IS NOT NULL AND discarded = 0 ORDER BY id DESC LIMIT ?",
    )
    .bind(before_id)
    .bind(limit)
    .fetch_all(k.pool())
    .await?;
    let mut lines: Vec<ReaderLine> = rows.iter().map(reader_line).collect();
    lines.reverse();
    Ok(lines)
}

/// Whether a line arrived in the last `within_secs` — "is a VN being read right
/// now", asked at the moment a lookup comes in.
///
/// Only looks backwards, because at write time there is no forward to look at.
/// A lookup fired in the seconds *before* a session's first line is therefore
/// dropped, which is the right way round to be wrong: the alternative admits
/// every lookup made while the reader was elsewhere.
pub async fn line_within(
    k: &Knowledge,
    now_ts: f64,
    within_secs: f64,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query(
        "SELECT EXISTS(
             SELECT 1 FROM lines WHERE discarded = 0 AND ts >= ? AND ts <= ?
         ) AS recent",
    )
    .bind(now_ts - within_secs)
    .bind(now_ts)
    .fetch_one(k.pool())
    .await?;
    Ok(row.get::<i64, _>("recent") == 1)
}

/// A line paired with the work it was stamped for, so a summary can be scoped
/// to one VN.
pub struct WorkedLine {
    pub event: LineEvent,
    pub work: Option<String>,
}

pub async fn fetch_worked_lines(
    k: &Knowledge,
    from_ts: f64,
    to_ts: f64,
) -> Result<Vec<WorkedLine>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT ts, chars, work FROM lines
             WHERE ts >= ? AND ts < ? AND discarded = 0 ORDER BY ts",
    )
    .bind(from_ts)
    .bind(to_ts)
    .fetch_all(k.pool())
    .await?;

    Ok(rows
        .iter()
        .map(|r| WorkedLine {
            event: LineEvent {
                ts: r.get("ts"),
                chars: r.get("chars"),
            },
            work: r.get("work"),
        })
        .collect())
}

pub async fn fetch_line_events(
    k: &Knowledge,
    from_ts: f64,
    to_ts: f64,
) -> Result<Vec<LineEvent>, sqlx::Error> {
    Ok(fetch_worked_lines(k, from_ts, to_ts)
        .await?
        .into_iter()
        .map(|c| c.event)
        .collect())
}

/// Every line's raw text with the work it belongs to, for the prose figures.
///
/// The whole stream in one query rather than one query per work: the point of
/// those figures is the comparison against everything *else* you have read, so
/// both sides come out of the same pass.
pub async fn fetch_line_texts(k: &Knowledge) -> Result<Vec<(String, Option<String>)>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT text, work FROM lines WHERE text IS NOT NULL AND discarded = 0 ORDER BY id",
    )
    .fetch_all(k.pool())
    .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get("text"), r.get("work")))
        .collect())
}

/// A line as the tokenizer needs it: the text to split, and the id that moves
/// the ingest watermark.
#[derive(Debug)]
pub struct IngestLine {
    pub id: i64,
    pub ts: f64,
    pub text: String,
    /// The work this line was stamped with, for the per-work sink. `None` for
    /// text read before a title was set, which is why `work_terms` can never
    /// account for quite everything `vocabulary` does.
    pub work: Option<String>,
}

pub async fn fetch_lines_after(
    k: &Knowledge,
    after_id: i64,
) -> Result<Vec<IngestLine>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, ts, text, work FROM lines
             WHERE id > ? AND text IS NOT NULL AND discarded = 0 ORDER BY id",
    )
    .bind(after_id)
    .fetch_all(k.pool())
    .await?;
    Ok(rows
        .iter()
        .map(|r| IngestLine {
            id: r.get("id"),
            ts: r.get("ts"),
            text: r.get("text"),
            work: r.get("work"),
        })
        .collect())
}

/// Every kept line's raw text, oldest first — the input to the kanji pass.
///
/// This is the one read that pulls all the text of all history into memory at
/// once. It is a few hundred kilobytes and the kanji tab is the only caller;
/// the derivations that run on every request deliberately use the columns-only
/// shapes above instead.
pub async fn fetch_kanji_lines(k: &Knowledge) -> Result<Vec<crate::stats::KanjiLine>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT ts, text, work FROM lines
         WHERE text IS NOT NULL AND discarded = 0 ORDER BY ts",
    )
    .fetch_all(k.pool())
    .await?;
    Ok(rows
        .iter()
        .map(|r| crate::stats::KanjiLine {
            ts: r.get("ts"),
            text: r.get("text"),
            work: r.get("work"),
        })
        .collect())
}
