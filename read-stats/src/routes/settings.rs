//! `/api/settings` and `/api/pause` — the knobs.

use axum::Json;
use axum::extract::State;
use chrono::NaiveDate;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::clock::now_ts;
use crate::db::{self, Settings};
use crate::error::AppError;

pub async fn get_settings(State(state): State<AppState>) -> Result<Json<Settings>, AppError> {
    Ok(Json(db::load_settings(&state.pool).await?))
}

/// Settings whose value is free text rather than a number. Everything else must
/// parse as one, which is what keeps a typo out of the derivation thresholds.
const TEXT_KEYS: &[&str] = &["current_work", "pace_start_date", "vn_window"];

pub async fn put_settings(
    State(state): State<AppState>,
    Json(updates): Json<serde_json::Map<String, Value>>,
) -> Result<Json<Settings>, AppError> {
    for (key, value) in &updates {
        if !db::SETTING_KEYS.contains(&key.as_str()) {
            return Err(AppError::BadRequest(format!("unknown setting: {key}")));
        }
        let stored = if TEXT_KEYS.contains(&key.as_str()) {
            let Some(s) = value.as_str() else {
                return Err(AppError::BadRequest(format!("{key} must be a string")));
            };
            let s = s.trim();
            if key == "pace_start_date" && !s.is_empty() && s.parse::<NaiveDate>().is_err() {
                return Err(AppError::BadRequest(format!("bad date: {s}")));
            }
            s.to_string()
        } else {
            let Some(num) = value.as_f64() else {
                return Err(AppError::BadRequest(format!("{key} must be a number")));
            };
            num.to_string()
        };
        db::save_setting(&state.pool, key, &stored).await?;
    }
    Ok(Json(db::load_settings(&state.pool).await?))
}

/// Toggle the tracking pause. Returns `{"paused": bool}`.
pub async fn toggle_pause(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let paused = db::toggle_pause(&state.pool, now_ts()).await?;
    Ok(Json(json!({ "paused": paused })))
}
