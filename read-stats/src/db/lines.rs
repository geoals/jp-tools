//! `lines` — the raw hooked line stream, and the four shapes it is read in.
//!
//! vn-ws-logger.py appends here as Textractor hooks each text box; read-stats
//! only ever reads and flags. Nothing is deleted: a line that shouldn't count
//! gets `discarded = 1` and every read filters it out.
//!
//! The four shapes exist because the callers genuinely need different columns:
//!
//! - [`ReaderLine`] — id + text, for the phone's live feed.
//! - [`ClassifiedLine`] / [`crate::stats::LineEvent`] — time + chars + the
//!   speech/prose split, for the derivations.
//! - [`crate::stats::WorkLine`] — time + chars + work, for per-VN totals.
//! - [`IngestLine`] — id + text, for tokenizing into `word_days`.
//!
//! Fetching all columns for all of them would be simpler and would also mean
//! dragging every line's text through the per-day aggregates.

use sqlx::{Row, SqlitePool};

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
    pool: &SqlitePool,
    after_id: i64,
    limit: i64,
) -> Result<Vec<ReaderLine>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, ts, chars, text FROM lines
         WHERE id > ? AND text IS NOT NULL AND discarded = 0 ORDER BY id LIMIT ?",
    )
    .bind(after_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(reader_line).collect())
}

/// The newest `limit` lines, oldest first — the backlog a reader gets on open
/// so the screen isn't blank until the next line is hooked.
pub async fn fetch_recent_lines(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<ReaderLine>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, ts, chars, text FROM lines
         WHERE text IS NOT NULL AND discarded = 0 ORDER BY id DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
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
    pool: &SqlitePool,
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
    let rows = q.bind(i64::from(!discarded)).fetch_all(pool).await?;
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
pub async fn max_line_id(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    let row = sqlx::query("SELECT COALESCE(MAX(id), 0) AS max_id FROM lines")
        .fetch_one(pool)
        .await?;
    Ok(row.get("max_id"))
}

/// A classified line paired with the work it was stamped for, so the dialogue
/// summary can scope its split to one VN. The 「」 classification comes from the
/// same scanner `fetch_line_events` uses.
pub struct ClassifiedLine {
    pub event: LineEvent,
    pub work: Option<String>,
}

pub async fn fetch_classified_lines(
    pool: &SqlitePool,
    from_ts: f64,
    to_ts: f64,
) -> Result<Vec<ClassifiedLine>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT ts, chars, text, work FROM lines
             WHERE ts >= ? AND ts < ? AND discarded = 0 ORDER BY ts",
    )
    .bind(from_ts)
    .bind(to_ts)
    .fetch_all(pool)
    .await?;

    // One scanner across the whole stream: a speech broken over several text
    // boxes leaves its 「 open on the first row, so depth has to carry. It is
    // dropped across a break too long for that to be what happened — see
    // `dialogue::CARRY_GAP_SECS`.
    let mut scanner = crate::dialogue::Scanner::new();
    let mut prev_ts: Option<f64> = None;
    Ok(rows
        .iter()
        .map(|r| {
            let ts: f64 = r.get("ts");
            let chars: i64 = r.get("chars");
            let text: Option<String> = r.get("text");
            let work: Option<String> = r.get("work");
            if prev_ts.is_some_and(|p| ts - p > crate::dialogue::CARRY_GAP_SECS) {
                scanner.reset();
            }
            prev_ts = Some(ts);
            let event = match text {
                Some(text) => {
                    let split = scanner.scan(&text);
                    LineEvent {
                        ts,
                        chars,
                        // `chars` is authoritative (startup recounts it), so
                        // clamp rather than let a stale disagreement make
                        // narration negative.
                        dialogue_chars: split.dialogue.min(chars),
                        classified: true,
                    }
                }
                None => {
                    scanner.reset();
                    LineEvent {
                        ts,
                        chars,
                        dialogue_chars: 0,
                        classified: false,
                    }
                }
            };
            ClassifiedLine { event, work }
        })
        .collect())
}

pub async fn fetch_line_events(
    pool: &SqlitePool,
    from_ts: f64,
    to_ts: f64,
) -> Result<Vec<LineEvent>, sqlx::Error> {
    Ok(fetch_classified_lines(pool, from_ts, to_ts)
        .await?
        .into_iter()
        .map(|c| c.event)
        .collect())
}

pub async fn fetch_work_lines(
    pool: &SqlitePool,
) -> Result<Vec<crate::stats::WorkLine>, sqlx::Error> {
    let rows = sqlx::query("SELECT ts, chars, work FROM lines WHERE discarded = 0 ORDER BY ts")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| crate::stats::WorkLine {
            ts: r.get("ts"),
            chars: r.get("chars"),
            work: r.get("work"),
        })
        .collect())
}

/// A line as the tokenizer needs it: the text to split, and the id that moves
/// the ingest watermark.
#[derive(Debug)]
pub struct IngestLine {
    pub id: i64,
    pub ts: f64,
    pub text: String,
}

pub async fn fetch_lines_after(
    pool: &SqlitePool,
    after_id: i64,
) -> Result<Vec<IngestLine>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, ts, text FROM lines
             WHERE id > ? AND text IS NOT NULL AND discarded = 0 ORDER BY id",
    )
    .bind(after_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| IngestLine {
            id: r.get("id"),
            ts: r.get("ts"),
            text: r.get("text"),
        })
        .collect())
}
