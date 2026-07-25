//! Clearing lines out of every derived figure, and undoing that.

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::info;

use crate::app::AppState;
use crate::db;
use crate::error::AppError;

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
