//! What the reading page can do right now, in one round trip on open.

use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::db;
use crate::error::AppError;

/// Everything the reader needs on open, in one round trip.
pub async fn reader_state(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let settings = db::load_settings(&state.local).await?;
    let caps = super::capabilities::probe(&state).await;
    Ok(Json(json!({
        "paused": settings.capture_paused,
        "current_work": settings.current_work,
        "capture_available": state.vn_capture_script.is_file(),
        "explain_available": state.anthropic_api_key.is_some(),
        // Quality-only: capture works without it, so the reader shows a hint
        // rather than disabling the mine button.
        "trim_available": caps["whisper"]["ok"],
        // What the feed groups lines into sessions by — the same gap the
        // dashboard's own session derivation uses, so a header here agrees
        // with what `#today` would call the same sitting.
        "session_gap_secs": settings.session_gap_secs,
        // Everything else this installation can or cannot do, one row each,
        // with the sentence that turns it on. See `reader::capabilities`.
        "capabilities": caps,
    })))
}
