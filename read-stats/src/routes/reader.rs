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

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// Record that the reader did something deliberate on this page just now, so
/// `stats::Presence` credits the surrounding gap even when no Yomitan lookup or
/// mined card landed in it (reading an explanation, mining without a fresh
/// lookup). Best-effort: a failed insert is logged, never propagated — the
/// action it accompanies must still succeed.
async fn mark_presence(state: &AppState, kind: &str) {
    if let Err(e) = db::insert_reader_mark(&state.pool, now_ts(), kind).await {
        warn!(error = %e, kind, "failed to record reader presence mark");
    }
}

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

    // Pressing mine proves presence whether or not the capture lands (a stale
    // line still fails), and it attaches media to an existing note rather than
    // creating one, so it never shows up as a card mark.
    mark_presence(&state, "mine").await;

    // Which window to screenshot. Without it the script grabs whatever has
    // focus — correct when mining from the phone (the VN never loses focus on
    // this machine), wrong from a browser on this machine, which is what would
    // be focused at the moment the button was pressed.
    //
    // The current work's own window comes first; the global `vn_window` setting
    // is a legacy fallback for setups that predate per-work windows.
    let vn_window = match db::current_work_vn_window(&state.pool).await {
        Ok(Some(w)) if !w.trim().is_empty() => w,
        _ => db::load_settings(&state.pool)
            .await
            .map(|s| s.vn_window)
            .unwrap_or_default(),
    };

    let mut cmd = tokio::process::Command::new(&script);
    cmd.env("VN_JSON", "1");
    // Left unset when empty so a VN_WINDOW inherited from the environment
    // still applies.
    if !vn_window.is_empty() {
        cmd.env("VN_WINDOW", &vn_window);
    }
    let run = cmd.output();
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

/// Candidate window titles for the `vn_window` setting.
///
/// The VN's window title can't be guessed from the work title (`素晴らしき日々`
/// vs `素晴らしき日々～不連続存在～`) and changes with every game, so the
/// dashboard offers a list to pick from rather than a blank text box.
pub async fn vn_windows() -> Result<Json<Value>, AppError> {
    let out = tokio::process::Command::new("xdotool")
        .args(["search", "--name", ".", "getwindowname", "%@"])
        .output()
        .await
        .map_err(|e| AppError::Upstream(format!("xdotool unavailable: {e}")))?;

    let mut names: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|n| !n.is_empty() && !is_helper_window(n))
        .map(str::to_string)
        .collect();
    names.sort();
    names.dedup();
    Ok(Json(json!({ "windows": names })))
}

/// Wine/Qt/IME scaffolding that is never the VN. Everything else is offered,
/// since guessing which of the real windows is the game is the user's call.
fn is_helper_window(name: &str) -> bool {
    const NOISE: &[&str] = &[
        "Default IME",
        "Input",
        "xsettingsd",
        "Chromium clipboard",
        "Fcitx5 Input Window",
    ];
    NOISE.contains(&name) || name.starts_with("Qt Selection Owner")
}

/// Cap on one clear request. The button clears what is on screen, and the
/// reader keeps `MAX_LINES` (300) of those.
const MAX_DISCARD: usize = 500;

#[derive(Deserialize)]
pub struct DiscardBody {
    pub ids: Vec<i64>,
}

/// Retroactively drop lines from every derived figure: the ones Textractor
/// hooks while you are still finding the route, or a stretch re-read after
/// skipping back, which would otherwise be counted twice.
///
/// Pause covers the same ground prospectively; this is for when you only
/// notice afterwards, which is most of the time. Nothing is deleted — the rows
/// keep their `discarded` flag and `undiscard_lines` puts them back.
pub async fn discard_lines(
    State(state): State<AppState>,
    Json(body): Json<DiscardBody>,
) -> Result<Json<Value>, AppError> {
    set_discarded(&state, body.ids, true).await
}

/// Undo for `discard_lines`, taking the ids it returned.
pub async fn undiscard_lines(
    State(state): State<AppState>,
    Json(body): Json<DiscardBody>,
) -> Result<Json<Value>, AppError> {
    set_discarded(&state, body.ids, false).await
}

async fn set_discarded(
    state: &AppState,
    ids: Vec<i64>,
    discarded: bool,
) -> Result<Json<Value>, AppError> {
    if ids.len() > MAX_DISCARD {
        return Err(AppError::BadRequest(format!(
            "at most {MAX_DISCARD} lines at a time, got {}",
            ids.len()
        )));
    }
    let changed = db::set_lines_discarded(&state.pool, &ids, discarded).await?;
    info!(count = changed.len(), discarded, "reader cleared lines");
    // No presence mark here on purpose: clearing is a *suppress* action, like
    // pause. It widens the gap so the removed line's span stops being credited
    // (junk route-finding lines, a re-read stretch) — a mark at clear-time would
    // re-credit exactly what the clear is there to remove.
    Ok(Json(json!({ "ids": changed })))
}

/// How long to wait for whisper-service before calling the trim unavailable.
/// Short: this is polled on the reader and a slow probe shouldn't stall the UI.
const WHISPER_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(800);

/// True if whisper-service answers its health check. Used only to light the
/// reader's trim indicator — a capture still succeeds without it, attaching the
/// VAD-trimmed clip rather than one narrowed to the mined sentence.
async fn whisper_reachable(state: &AppState) -> bool {
    let url = format!("{}/health", state.whisper_url.trim_end_matches('/'));
    matches!(
        state.http.get(&url).timeout(WHISPER_PROBE_TIMEOUT).send().await,
        Ok(resp) if resp.status().is_success()
    )
}

/// Everything the reader needs on open, in one round trip.
pub async fn reader_state(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let settings = db::load_settings(&state.pool).await?;
    Ok(Json(json!({
        "paused": db::is_pause_open(&state.pool).await?,
        "current_work": settings.current_work,
        "capture_available": state.vn_capture_script.is_file(),
        "explain_available": state.anthropic_api_key.is_some(),
        // Quality-only: capture works without it, so the reader shows a hint
        // rather than disabling the mine button.
        "trim_available": whisper_reachable(&state).await,
    })))
}

/// Recent lines and, optionally, the word the reader has selected in the last
/// one. The lines are oldest-first with the target line last, mirroring what
/// the feed shows on screen so the server never has to guess which line is "the
/// current one".
#[derive(Deserialize)]
pub struct ExplainBody {
    pub context: Vec<String>,
    #[serde(default)]
    pub focus: String,
}

/// Enough earlier lines to place a pronoun or an unstated subject without
/// paying for a whole scene. The client sends what is on screen; this caps it.
const MAX_EXPLAIN_CONTEXT: usize = 12;

/// Ask the model for a short read on the line currently being read, centred on
/// a selected word if one was passed. Off unless an API key is configured.
pub async fn explain_line(
    State(state): State<AppState>,
    Json(body): Json<ExplainBody>,
) -> Result<Json<Value>, AppError> {
    let Some(api_key) = state.anthropic_api_key.clone() else {
        return Err(AppError::BadRequest(
            "no Anthropic API key set (JP_TOOLS_ANTHROPIC_API_KEY)".into(),
        ));
    };

    let mut context: Vec<String> = body
        .context
        .into_iter()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if context.is_empty() {
        return Err(AppError::BadRequest("no line to explain".into()));
    }
    if context.len() > MAX_EXPLAIN_CONTEXT {
        context.drain(0..context.len() - MAX_EXPLAIN_CONTEXT);
    }

    mark_presence(&state, "explain").await;
    let text = crate::llm::explain(
        &state.http,
        &api_key,
        &state.llm_model,
        &context,
        body.focus.trim(),
    )
    .await?;
    Ok(Json(json!({ "text": text })))
}
