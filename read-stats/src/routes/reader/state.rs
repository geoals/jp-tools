//! What the reading page can do right now, in one round trip on open.

use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::db;
use crate::error::AppError;

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
    let settings = db::load_settings(&state.local).await?;
    Ok(Json(json!({
        "paused": db::is_pause_open(&state.local).await?,
        "current_work": settings.current_work,
        "capture_available": state.vn_capture_script.is_file(),
        "explain_available": state.anthropic_api_key.is_some(),
        // Quality-only: capture works without it, so the reader shows a hint
        // rather than disabling the mine button.
        "trim_available": whisper_reachable(&state).await,
    })))
}
