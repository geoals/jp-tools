//! `/api/settings` and `/api/capture/pause` — the knobs.

use axum::Json;
use axum::extract::State;
use chrono::NaiveDate;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::db::{self, Settings};
use crate::error::AppError;

/// The stored settings plus the one fact the database cannot hold: whether a key
/// is coming from the environment. Every handler that answers with `Settings`
/// goes through this, so the two surfaces cannot disagree about whether a key
/// is configured.
async fn settings_for(state: &AppState) -> Result<Settings, AppError> {
    let mut settings = db::load_settings(&state.local).await?;
    settings.llm_key_from_env = state
        .env_api_key
        .as_deref()
        .is_some_and(|k| !k.trim().is_empty());
    Ok(settings)
}

pub async fn get_settings(State(state): State<AppState>) -> Result<Json<Settings>, AppError> {
    Ok(Json(settings_for(&state).await?))
}

/// Settings whose value is free text rather than a number. Everything else must
/// parse as one, which is what keeps a typo out of the derivation thresholds.
const TEXT_KEYS: &[&str] = &[
    "current_work",
    "pace_start_date",
    "vn_window",
    "line_source",
    "line_source_ws_url",
    "llm_provider",
    "llm_base_url",
    "llm_model",
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
            // Same reason as `line_source`: a name nothing speaks would be found
            // only by pressing a button and reading a failure.
            if key == "llm_provider" && jp_mine_core::llm::Kind::parse(s).is_none() {
                return Err(AppError::BadRequest(format!(
                    "llm_provider must be anthropic or openai, not {s}"
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
    Ok(Json(settings_for(&state).await?))
}

/// `PUT /api/settings/llm-key` — store the model API key, or clear it.
///
/// Its own endpoint rather than a key in [`put_settings`], because the value must
/// never come back out: `GET /api/settings` serializes the whole `Settings`
/// struct to whoever asks, and the server binds `0.0.0.0`. `settings.llm_has_key`
/// is the only thing a client learns about it.
///
/// The reply says whether the key *works*, not merely that it was written. A key
/// pasted with a character missing is the ordinary mistake here, and finding out
/// from a failed explain button three lines later is what the sentence in the
/// dialog exists to prevent.
pub async fn put_llm_key(
    State(state): State<AppState>,
    Json(body): Json<LlmKeyBody>,
) -> Result<Json<Value>, AppError> {
    let key = body.api_key.trim();
    db::save_setting(&state.local, db::LLM_API_KEY, key).await?;
    if key.is_empty() {
        return Ok(Json(json!({ "ok": true, "detail": "key cleared" })));
    }
    match crate::services::llm::check(&state).await {
        Ok(model) => Ok(Json(
            json!({ "ok": true, "detail": format!("{model} answered") }),
        )),
        // Stored anyway, and reported. A key that cannot reach the network right
        // now is still the key the reader meant to save, and clearing it behind
        // their back would make a working key look rejected.
        Err(e) => Ok(Json(json!({ "ok": false, "detail": e.to_string() }))),
    }
}

#[derive(serde::Deserialize)]
pub struct LlmKeyBody {
    /// Empty clears the stored key.
    pub api_key: String,
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
