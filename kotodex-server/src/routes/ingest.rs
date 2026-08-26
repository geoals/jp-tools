//! `POST /api/lines` — the ledger's front door for captured text.
//!
//! Every source goes through here: the Textractor logger beside a VN, a bridge
//! from another mining tool, a phone reading an epub. Writing the table
//! directly is what the `ruby` column drifting across three definitions came
//! from, and a source on a phone could not do it anyway.
//!
//! What the source owns is turning its own capture into a line — the hooker's
//! junk, a continuation split across two text boxes, its own dedup. What the
//! ledger owns is everything derived: the character count, which work the line
//! belongs to, whether capture is paused at all.

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{info, warn};

use crate::app::AppState;
use crate::db;
use crate::error::AppError;

/// Cap on one batch. A source holding lines through an outage flushes them in
/// several requests rather than one unbounded transaction.
const MAX_BATCH: usize = 500;

/// Cap on a single line, matching what the sources already normalise to. A
/// longer one is a hook pointed at the wrong address, not a sentence.
const MAX_TEXT: usize = 4000;

/// Sources are free-form so a new one needs no change here, but the column is
/// read back by `retract` and by the per-source figures, so it has to be a
/// plain token rather than whatever arrived.
const MAX_SOURCE: usize = 32;

#[derive(Deserialize)]
pub struct IngestBody {
    /// Where the text came from: `vn`, or a name the bridge picks for itself.
    #[serde(default = "default_source")]
    pub source: String,
    /// Work for every line in the batch that doesn't name its own. Falls back
    /// to the dashboard's "now reading" when neither does.
    pub work: Option<String>,
    #[serde(default)]
    pub lines: Vec<IncomingLine>,
    /// The source's own health, for the capture badge. A source with nothing
    /// to send posts this alone to say it is still attached.
    pub status: Option<SourceStatus>,
}

#[derive(Deserialize)]
pub struct IncomingLine {
    pub text: String,
    /// Epoch seconds at capture. Server time when absent — only a source
    /// replaying a backlog knows better.
    pub ts: Option<f64>,
    pub work: Option<String>,
    /// Furigana as `[[start, len, reading], ...]` over `text` in UTF-16 code
    /// units. Stored as given: nothing here has an opinion about a reading.
    pub ruby: Option<Value>,
}

#[derive(Deserialize)]
pub struct SourceStatus {
    /// Something is actually feeding the source — the hooker's socket is up,
    /// the clipboard watcher is polling.
    pub attached: bool,
    /// Lines captured but not yet accepted here.
    #[serde(default)]
    pub pending: i64,
}

fn default_source() -> String {
    "vn".to_string()
}

/// Accept a batch of captured lines.
///
/// Returns the ids assigned, so a source can retract the last one when the
/// next capture turns out to continue it.
pub async fn ingest_lines(
    State(state): State<AppState>,
    Json(body): Json<IngestBody>,
) -> Result<Json<Value>, AppError> {
    let source = clean_source(&body.source)?;
    if body.lines.len() > MAX_BATCH {
        return Err(AppError::BadRequest(format!(
            "at most {MAX_BATCH} lines at a time, got {}",
            body.lines.len()
        )));
    }

    let settings = db::load_settings(&state.local).await.unwrap_or_default();

    if let Some(status) = &body.status {
        write_heartbeat(&state, &source, status).await;
    }

    // Pause is a ledger-level decision, not the source's: a bridge on another
    // machine cannot watch the setting, and a source that could would still be
    // a second implementation of the rule. Accepted and dropped rather than
    // refused — a 4xx would make a source hold lines and flush them the moment
    // capture resumed, which is the opposite of what pausing is for.
    if settings.capture_paused {
        return Ok(Json(json!({ "ids": [], "paused": true })));
    }

    let now = crate::clock::now_ts();
    let fallback_work = body
        .work
        .as_deref()
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .or_else(|| (!settings.current_work.is_empty()).then(|| settings.current_work.clone()));

    let lines: Vec<db::NewLine> = body
        .lines
        .into_iter()
        .filter_map(|line| {
            let text = line.text.trim();
            // A line that is nothing but whitespace carries no reading and no
            // voice. Punctuation-only lines are kept: 「……」 is a real text box,
            // and it counts zero characters on its own.
            if text.is_empty() {
                return None;
            }
            Some(db::NewLine {
                ts: line.ts.unwrap_or(now),
                text: text.chars().take(MAX_TEXT).collect(),
                work: line
                    .work
                    .filter(|w| !w.is_empty())
                    .or_else(|| fallback_work.clone()),
                ruby: line.ruby,
            })
        })
        .collect();

    let ids = db::insert_lines(&state.knowledge, &source, &lines).await?;
    if !ids.is_empty() {
        info!(count = ids.len(), source, "ingested lines");
    }
    Ok(Json(json!({ "ids": ids, "paused": false })))
}

#[derive(Deserialize)]
pub struct RetractBody {
    #[serde(default = "default_source")]
    pub source: String,
}

/// Take the newest line from a source back out of the feed — the continuation
/// case, where the box that followed turned out to be the rest of the same
/// sentence and the joined line replaces both.
pub async fn retract_line(
    State(state): State<AppState>,
    Json(body): Json<RetractBody>,
) -> Result<Json<Value>, AppError> {
    let source = clean_source(&body.source)?;
    let id = db::retract_last_line(&state.knowledge, &source).await?;
    Ok(Json(json!({ "id": id })))
}

fn clean_source(source: &str) -> Result<String, AppError> {
    let source = source.trim();
    if source.is_empty() || source.len() > MAX_SOURCE {
        return Err(AppError::BadRequest(format!(
            "source must be 1..={MAX_SOURCE} characters"
        )));
    }
    if !source
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(AppError::BadRequest(
            "source may only hold letters, digits, - and _".to_string(),
        ));
    }
    Ok(source.to_string())
}

/// Publish the source's health where the capture badge reads it.
///
/// One key rather than one per source: the badge answers "is capture working"
/// for the sitting in progress, and two sources feeding one sitting are one
/// answer. A failure to write it is not a failure to ingest — the lines are
/// the point, the badge is not.
async fn write_heartbeat(state: &AppState, source: &str, status: &SourceStatus) {
    let beat = json!({
        "ts": crate::clock::now_ts(),
        "ws": status.attached,
        "pending": status.pending,
        "source": source,
    });
    if let Err(e) = db::save_setting(&state.local, "vn_logger_heartbeat", &beat.to_string()).await {
        warn!(error = %e, "capture heartbeat unwritable");
    }
}
