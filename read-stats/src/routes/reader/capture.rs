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
    capture::run(&state).await.map(Json)
}

/// Window titles to choose the capture target from.
pub async fn vn_windows() -> Result<Json<Value>, AppError> {
    Ok(Json(json!({ "windows": capture::list_windows().await? })))
}
