//! `lines` — the raw hooked line stream, and the four shapes it is read in.
//!
//! Sources append through `POST /api/lines`; nothing outside this module
//! writes the table. Nothing is deleted either: a line that shouldn't count
//! gets `discarded = 1` and every read filters it out.
//!
//! Four shapes, because the callers need different columns and fetching them
//! all would drag every line's text through the per-day aggregates:
//!
//! - [`ReaderLine`] — id + text, for the reading view's live feed.
//! - [`WorkedLine`] / [`crate::stats::LineEvent`] — time + chars, for the
//!   derivations.
//! - [`crate::stats::WorkLine`] — time + chars + work, for per-VN totals.
//! - [`IngestLine`] — id + text, for tokenizing into `word_days`.
//! - [`NewLine`] — what a source hands over, on the way in.

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
    /// Furigana the game drew with the line, as `[[start, len, reading], ...]`
    /// over `text` in UTF-16 code units — the same units as
    /// [`jp_core::highlight::Span`], so the overlay indexes both alike. Passed
    /// through as stored: only the overlay reads it, and nothing here has an
    /// opinion about a reading.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ruby: Option<serde_json::Value>,
}

/// Lines newer than `after_id`, oldest first. The reader's SSE loop calls this
/// on a short interval, so it stays a bounded index range scan.
pub async fn fetch_lines_after_id(
    k: &Knowledge,
    after_id: i64,
    limit: i64,
) -> Result<Vec<ReaderLine>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, ts, chars, text, ruby FROM lines
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
        "SELECT id, ts, chars, text, ruby FROM lines
         WHERE text IS NOT NULL AND discarded = 0 ORDER BY id DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(k.pool())
    .await?;
    let mut lines: Vec<ReaderLine> = rows.iter().map(reader_line).collect();
    lines.reverse();
    Ok(lines)
}

/// Flag lines as not-reading (`discarded = 1`) or put them back — how a line
/// stops counting without leaving the raw table.
///
/// Ids come from the client rather than being a "last N" computed here: the
/// reader is clearing what is on screen, and a line hooked between the tap and
/// the request must not be swept up with it. Returns the ids actually changed,
/// which is what undo re-sends.
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
        ruby: r
            .get::<Option<String>, _>("ruby")
            .and_then(|s| serde_json::from_str(&s).ok()),
    }
}

/// Highest line id currently stored, or 0 when the table is empty.
pub async fn max_line_id(k: &Knowledge) -> Result<i64, sqlx::Error> {
    let row = sqlx::query("SELECT COALESCE(MAX(id), 0) AS max_id FROM lines")
        .fetch_one(k.pool())
        .await?;
    Ok(row.get("max_id"))
}

/// Every line of the sitting still in progress, oldest first.
///
/// The boundary is the one `stats::derive_sessions` splits on: walking back from
/// the newest line, the first gap over `session_gap_secs` ends it. Derived here
/// because there is no sessions table — a session is a shape the line stream is
/// read in, never a stored row.
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
        "SELECT id, ts, chars, text, ruby FROM lines
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
        "SELECT id, ts, chars, text, ruby FROM lines
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

/// The text of a handful of specific lines, keyed by id.
///
/// For `term_surfaces`' example lines: a triage row asks for the sentence
/// behind each spelling, which is a few ids, not a range. Discarded lines are
/// included — the word was still read, and hiding the evidence would leave the
/// spelling with a count and nothing to show for it.
pub async fn fetch_line_texts_by_id(
    k: &Knowledge,
    ids: &[i64],
) -> Result<std::collections::HashMap<i64, String>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let places = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!("SELECT id, text FROM lines WHERE id IN ({places}) AND text IS NOT NULL");
    let mut q = sqlx::query(&sql);
    for id in ids {
        q = q.bind(id);
    }
    Ok(q.fetch_all(k.pool())
        .await?
        .iter()
        .map(|r| (r.get("id"), r.get("text")))
        .collect())
}

/// A line as a source hands it over, ready to store.
///
/// `chars` is not on it: the count is the ledger's rule
/// ([`jp_core::text::chars::count_chars`]), not the source's, so a new source cannot
/// drift the figure every rate is derived from.
pub struct NewLine {
    pub ts: f64,
    pub text: String,
    pub work: Option<String>,
    pub ruby: Option<serde_json::Value>,
}

/// Append captured lines, returning their ids in the order given.
///
/// One transaction for the batch: a source that has been holding lines through
/// an outage flushes them as a unit, and a partial flush would leave it unable
/// to say where to resume from.
pub async fn insert_lines(
    k: &Knowledge,
    source: &str,
    lines: &[NewLine],
) -> Result<Vec<i64>, sqlx::Error> {
    if lines.is_empty() {
        return Ok(Vec::new());
    }
    let mut tx = k.pool().begin().await?;
    let mut ids = Vec::with_capacity(lines.len());
    for line in lines {
        let ruby = line
            .ruby
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_default());
        let row = sqlx::query(
            "INSERT INTO lines (ts, chars, text, source, work, discarded, ruby)
             VALUES (?, ?, ?, ?, ?, 0, ?) RETURNING id",
        )
        .bind(line.ts)
        .bind(jp_core::text::chars::count_chars(&line.text))
        .bind(&line.text)
        .bind(source)
        .bind(line.work.as_deref())
        .bind(ruby)
        .fetch_one(&mut *tx)
        .await?;
        ids.push(row.get("id"));
    }
    tx.commit().await?;
    Ok(ids)
}

/// Flag the newest line from `source` as discarded — a source taking back a
/// line it has already handed over, because the next capture turned out to
/// continue it.
///
/// Discarded rather than deleted, for the same reason as
/// [`set_lines_discarded`]: an id already handed to `term_surfaces` or crossed
/// by an ingest watermark has to stay resolvable.
pub async fn retract_last_line(k: &Knowledge, source: &str) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query(
        "UPDATE lines SET discarded = 1
         WHERE id = (SELECT MAX(id) FROM lines WHERE source = ?) RETURNING id",
    )
    .bind(source)
    .fetch_optional(k.pool())
    .await?;
    Ok(row.map(|r| r.get("id")))
}
