//! Clearing lines out of every derived figure, and undoing that; and paging
//! further back into the line history than the live feed keeps in memory.

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::info;

use super::highlight;
use crate::app::AppState;
use crate::db;
use crate::error::AppError;

/// Cap on one clear request. The button clears what is on screen.
const MAX_DISCARD: usize = 500;

/// Cap on one backscroll page, so a client that scrolls to the very top of a
/// long history doesn't pull it in one request.
const MAX_HISTORY_PAGE: i64 = 500;
const DEFAULT_HISTORY_PAGE: i64 = 200;

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
    let changed = db::set_lines_discarded(&state.knowledge, &ids, discarded).await?;
    info!(count = changed.len(), discarded, "reader cleared lines");
    // No presence mark here on purpose: clearing is a *suppress* action, like
    // pause. It widens the gap so the removed line's span stops being credited
    // (junk route-finding lines, a re-read stretch) — a mark at clear-time would
    // re-credit exactly what the clear is there to remove.
    Ok(Json(json!({ "ids": changed })))
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    /// The oldest line id currently on screen — this page picks up before it.
    pub before: i64,
    pub limit: Option<i64>,
}

/// One page of lines older than `before`, tokenised exactly as the live
/// stream tokenises them, for the reader to prepend when a scroll back
/// reaches the top of what it already holds.
pub async fn lines_before(
    State(state): State<AppState>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = q
        .limit
        .unwrap_or(DEFAULT_HISTORY_PAGE)
        .clamp(1, MAX_HISTORY_PAGE);
    let lines = db::fetch_lines_before_id(&state.knowledge, q.before, limit).await?;
    let hl = highlight::shared(&state).await;
    let mut out = Vec::with_capacity(lines.len());
    for line in &lines {
        let tokens = match hl.as_deref() {
            Some(h) => highlight::spans(&state.knowledge, h, &line.text).await,
            None => Vec::new(),
        };
        out.push(json!({
            "id": line.id,
            "ts": line.ts,
            "chars": line.chars,
            "text": line.text,
            "tokens": tokens,
        }));
    }
    Ok(Json(json!({ "lines": out })))
}
