//! The phone-side reading view: a live feed of the lines Textractor hooks,
//! plus the button that fires vn-capture.sh on this machine.
//!
//! The VN, Textractor and Anki all stay on the PC; the phone shows the line
//! stream in a browser so Yomitan can scan it (its AnkiConnect endpoint is
//! `/anki-proxy`, which forwards to the PC's Anki — the same collection
//! vn-capture.sh attaches media to).
//!
//! Lines are read out of the `lines` table rather than from Textractor's
//! WebSocket directly: vn-ws-logger.py is already writing them there, and its
//! WS plugin can crash Textractor when a client disconnects abortively, so a
//! second WS client is a risk with nothing to gain.

use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_core::Stream;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{info, warn};

use crate::app::AppState;
use crate::db;
use crate::error::AppError;

/// How often the stream checks for new lines, which is the whole of the
/// pipeline's controllable latency: vn-ws-logger.py commits in autocommit mode
/// the moment Textractor hooks a line, and the LAN hop is sub-millisecond. A
/// poll of N ms therefore costs a uniform 0..N delay — 250ms measured a mean of
/// 108ms, which reads as perceptibly behind the voice.
///
/// 30ms puts the mean at ~15ms, below the threshold where the line looks like
/// it lags the VN. The cost is ~33 queries/sec per connected reader, each an
/// index seek past the end of `lines` returning nothing — WAL readers don't
/// block the logger's writes, so this is not a contention risk either.
const POLL_INTERVAL: Duration = Duration::from_millis(30);

/// Lines shown on open when the client isn't resuming.
const DEFAULT_BACKLOG: i64 = 40;

/// Cap on a single catch-up batch, so a client resuming after hours away
/// doesn't pull the whole history in one go.
const MAX_BATCH: i64 = 500;

/// vn-capture.sh runs VAD and (usually) a whisper transcription for the
/// sentence trim, so it is slow by design. Past this it is stuck, not working.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Deserialize)]
pub struct StreamQuery {
    /// Resume after this line id instead of sending a backlog.
    pub after: Option<i64>,
    pub backlog: Option<i64>,
}

/// Server-sent events, one per hooked line, `data` being the line JSON.
///
/// Each event carries its line id, so a browser that drops the connection
/// (phone screen off, tab backgrounded) reconnects with `Last-Event-ID` and
/// resumes exactly where it left off rather than replaying the backlog.
pub async fn lines_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<StreamQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let resume = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok())
        .or(q.after);
    let backlog = q.backlog.unwrap_or(DEFAULT_BACKLOG).clamp(0, MAX_BATCH);

    let stream = async_stream::stream! {
        // Opening batch: everything missed since `resume`, or the tail of the
        // log for a fresh client.
        let mut last_id = match resume {
            Some(id) => id,
            None => match db::fetch_recent_lines(&state.pool, backlog).await {
                Ok(lines) => {
                    let last = lines.last().map(|l| l.id);
                    for line in &lines {
                        yield Ok(line_event(line));
                    }
                    match last {
                        Some(id) => id,
                        // Empty table: start from the current end so the first
                        // hooked line still arrives.
                        None => db::max_line_id(&state.pool).await.unwrap_or(0),
                    }
                }
                Err(e) => {
                    warn!(error = %e, "reader backlog failed");
                    0
                }
            },
        };

        loop {
            match db::fetch_lines_after_id(&state.pool, last_id, MAX_BATCH).await {
                Ok(lines) => {
                    for line in &lines {
                        last_id = line.id;
                        yield Ok(line_event(line));
                    }
                }
                // A transient DB error must not end the stream — the client
                // would reconnect into the same error anyway.
                Err(e) => warn!(error = %e, "reader poll failed"),
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    };

    // Comment pings keep the connection open through the phone's idle timeouts.
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

fn line_event(line: &db::ReaderLine) -> Event {
    // json_data only fails on non-serializable values; ReaderLine is plain data.
    Event::default()
        .id(line.id.to_string())
        .json_data(line)
        .unwrap_or_else(|_| Event::default().comment("unserializable line"))
}

/// Run vn-capture.sh and hand its result back to the phone.
///
/// The script normally reports through notify-send on the PC desktop, which
/// nobody is looking at when reading from the phone — `VN_JSON=1` makes it
/// print a result object instead, and that becomes this response.
pub async fn vn_capture(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let script = state.vn_capture_script.clone();
    if !script.is_file() {
        return Err(AppError::BadRequest(format!(
            "vn-capture.sh not found at {} (set JP_TOOLS_VN_CAPTURE_SH)",
            script.display()
        )));
    }

    let run = tokio::process::Command::new(&script)
        .env("VN_JSON", "1")
        .output();
    let out = match tokio::time::timeout(CAPTURE_TIMEOUT, run).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            return Err(AppError::Upstream(format!(
                "could not run vn-capture.sh: {e}"
            )));
        }
        Err(_) => {
            return Err(AppError::Upstream(format!(
                "vn-capture.sh timed out after {}s",
                CAPTURE_TIMEOUT.as_secs()
            )));
        }
    };

    let stdout = String::from_utf8_lossy(&out.stdout);
    let Some(parsed) = stdout.lines().rev().find_map(|l| {
        serde_json::from_str::<Value>(l)
            .ok()
            .filter(Value::is_object)
    }) else {
        // No parseable result: surface the script's own diagnostics, which is
        // all there is to go on (a missing dependency, a broken ring buffer).
        let stderr = String::from_utf8_lossy(&out.stderr);
        let detail = [stderr.trim(), stdout.trim()]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or("no output")
            .lines()
            .next_back()
            .unwrap_or("no output")
            .to_string();
        return Err(AppError::Upstream(format!(
            "vn-capture.sh failed: {detail}"
        )));
    };

    // A failed capture is a normal outcome (stale line, Anki closed), not an
    // HTTP error — the reader shows the message and you press again.
    if parsed.get("ok").and_then(Value::as_bool) == Some(true) {
        info!(result = %parsed, "vn-capture succeeded");
    } else {
        warn!(result = %parsed, "vn-capture reported failure");
    }
    Ok(Json(parsed))
}

/// Everything the reader needs on open, in one round trip.
pub async fn reader_state(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let settings = db::load_settings(&state.pool).await?;
    Ok(Json(json!({
        "paused": db::is_pause_open(&state.pool).await?,
        "current_work": settings.current_work,
        "capture_available": state.vn_capture_script.is_file(),
    })))
}
