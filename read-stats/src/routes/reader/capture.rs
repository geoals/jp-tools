//! The mine button, and the window picker that aims it.

use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::AppError;
use crate::services::capture;

/// Run vn-capture.sh and hand its result back to the browser that asked.
pub async fn vn_capture(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    // Pressing mine proves presence whether or not the capture lands (a stale
    // line still fails), and it attaches media to an existing note rather than
    // creating one, so it never shows up as a card mark.
    super::mark_presence(&state, "mine").await;
    // No anchor and no note id: the button fires immediately, so "the newest
    // line" and "the note added last" are exactly what it means.
    capture::run(&state, capture::Target::default())
        .await
        .map(Json)
}

/// Window titles to choose the capture target from.
pub async fn vn_windows() -> Result<Json<Value>, AppError> {
    Ok(Json(json!({ "windows": capture::list_windows().await? })))
}

/// `GET /api/vn/window` — which window is the VN right now.
///
/// For `vn-capture.sh`, which is fired by hotkey and so has no caller to pass
/// it in. It resolved the same two rows in its own SQL before, which made the
/// script a second implementation of a rule that must have exactly one.
pub async fn vn_window(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "window": capture::vn_window(&state).await }))
}
