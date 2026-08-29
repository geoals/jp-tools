//! Which window is the game, and the picker that says so.

use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::db;
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
/// it in. The script resolving those rows in its own SQL would be a second
/// implementation of a rule that must have exactly one.
pub async fn vn_window(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "window": capture::vn_window(&state).await }))
}

/// `PUT /api/vn/window` — point the current work at a window.
///
/// For a surface that knows which window it is looking at but not the work's
/// row id: the overlay has neither the library nor the id, and the rule that
/// the window belongs to the *current* work is already this module's. The
/// dashboard's edit dialog writes the same column by id, because there it is
/// editing a work that may not be the one being read.
///
/// Upserts, so the first thing a reader does with a title the tracker stamped
/// is not blocked on the work having a metadata row yet.
pub async fn set_vn_window(
    State(state): State<AppState>,
    Json(req): Json<SetWindowReq>,
) -> Result<Json<Value>, AppError> {
    let settings = db::load_settings(&state.local).await?;
    let title = settings.current_work.trim();
    if title.is_empty() {
        return Err(AppError::BadRequest(
            "no work is being read, so there is nothing to set the window on".into(),
        ));
    }
    let work = db::upsert_work(&state.knowledge, title).await?;
    let window = req.window.trim();
    db::set_work_vn_window(
        &state.knowledge,
        work.id,
        (!window.is_empty()).then_some(window),
    )
    .await?;
    Ok(Json(json!({ "window": window })))
}

#[derive(serde::Deserialize)]
pub struct SetWindowReq {
    /// Empty clears it — the same meaning the work editor's box has.
    pub window: String,
}
