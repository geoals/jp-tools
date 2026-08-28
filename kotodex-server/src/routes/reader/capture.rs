//! Which window is the game, and the picker that says so.

use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::AppError;
use crate::services::{capture, desktop};

/// Window titles to choose the capture target from, and the one in front.
///
/// `focused` is answered beside the list because it is the whole interaction on
/// a good day: the reader is looking at the game when they open this, so one
/// button beats reading a list of thirty titles. The list stays for the case it
/// is wrong — the game is behind the browser, or has not started yet.
pub async fn vn_windows() -> Result<Json<Value>, AppError> {
    Ok(Json(json!({
        "windows": desktop::open_windows().await?,
        "focused": desktop::focused_window().await?,
    })))
}

/// `GET /api/vn/window` — which window is the VN right now.
///
/// For `vn-capture.sh`, which can be run on its own and so has no caller to pass
/// it in. It resolved the same two rows in its own SQL before, which made the
/// script a second implementation of a rule that must have exactly one.
pub async fn vn_window(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "window": capture::vn_window(&state).await }))
}
