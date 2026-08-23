//! `/api/settings` and `/api/capture/pause` — the knobs.

use axum::Json;
use axum::extract::State;
use chrono::NaiveDate;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::db::{self, Settings};
use crate::error::AppError;

pub async fn get_settings(State(state): State<AppState>) -> Result<Json<Settings>, AppError> {
    Ok(Json(db::load_settings(&state.local).await?))
}

/// Settings whose value is free text rather than a number. Everything else must
/// parse as one, which is what keeps a typo out of the derivation thresholds.
const TEXT_KEYS: &[&str] = &[
    "current_work",
    "pace_start_date",
    "vn_window",
    "line_source",
    "line_source_ws_url",
];

pub async fn put_settings(
    State(state): State<AppState>,
    Json(updates): Json<serde_json::Map<String, Value>>,
) -> Result<Json<Settings>, AppError> {
    for (key, value) in &updates {
        if !db::SETTING_KEYS.contains(&key.as_str()) {
            return Err(AppError::BadRequest(format!("unknown setting: {key}")));
        }
        let stored = if db::BOOL_SETTING_KEYS.contains(&key.as_str()) {
            let Some(b) = value.as_bool() else {
                return Err(AppError::BadRequest(format!("{key} must be true or false")));
            };
            if b { "1" } else { "0" }.to_string()
        } else if TEXT_KEYS.contains(&key.as_str()) {
            let Some(s) = value.as_str() else {
                return Err(AppError::BadRequest(format!("{key} must be a string")));
            };
            let s = s.trim();
            if key == "pace_start_date" && !s.is_empty() && s.parse::<NaiveDate>().is_err() {
                return Err(AppError::BadRequest(format!("bad date: {s}")));
            }
            // A typo here stops every line arriving, and the producer polls
            // this rather than being told, so nothing would report the mistake.
            if key == "line_source" && !matches!(s, "ws" | "clipboard") {
                return Err(AppError::BadRequest(format!(
                    "line_source must be ws or clipboard, not {s}"
                )));
            }
            s.to_string()
        } else {
            let Some(num) = value.as_f64() else {
                return Err(AppError::BadRequest(format!("{key} must be a number")));
            };
            num.to_string()
        };
        db::save_setting(&state.local, key, &stored).await?;
    }
    Ok(Json(db::load_settings(&state.local).await?))
}

/// Toggle capture. Returns `{"paused": bool}`.
///
/// This does not filter anything: it flips `settings.capture_paused`, which
/// vn-ws-logger.py polls and answers by closing its Textractor WebSocket. While
/// it is set, no line reaches the stream at all — which is why there is nothing
/// to exclude on read and no interval log to keep.
pub async fn toggle_capture(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let paused = !db::load_settings(&state.local).await?.capture_paused;
    db::save_setting(
        &state.local,
        "capture_paused",
        if paused { "1" } else { "0" },
    )
    .await?;
    Ok(Json(json!({ "paused": paused })))
}
