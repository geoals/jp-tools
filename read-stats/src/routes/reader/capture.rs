//! Which window is the game, and the picker that says so.

use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::AppError;
use crate::services::capture;

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
