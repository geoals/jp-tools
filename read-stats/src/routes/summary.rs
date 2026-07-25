//! `/api/summary` — the top of the dashboard: today against the goal, and the
//! streak.

use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::db;
use crate::error::AppError;
use crate::history::History;
use crate::routes::json::focus_json;
use crate::stats;

pub async fn summary(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let h = History::load(&state).await?;
    let today = h.today;

    let (vn, manual) = h.day_maps();
    let total = h.total_days();

    let today_total = total.get(&today).copied().unwrap_or_default();
    let today_vn = vn.get(&today).copied().unwrap_or_default();
    let today_manual = manual.get(&today).copied().unwrap_or_default();

    let (current, best) = stats::streaks(&total, h.settings.goal_floor_mins as f64 * 60.0, today);

    let day_start = h.day_start(today);
    let today_lookups = h.lookup_days().get(&today).copied().unwrap_or(0);
    let today_focus = h.focus_days().get(&today).copied().unwrap_or_default();

    Ok(Json(json!({
        "paused": db::is_pause_open(&state.pool).await?,
        "today": {
            "date": today.to_string(),
            "chars": today_total.chars,
            "active_secs": today_total.active_secs,
            "vn": today_vn,
            "manual": today_manual,
            "cards": h.cards_in(day_start, day_start + 86400.0),
            "lookups": today_lookups,
            "lookups_per_1k": stats::rate::lookups_per_1k(today_lookups, today_total.chars),
            "focus": focus_json(&today_focus),
        },
        "goal": {
            "floor_mins": h.settings.goal_floor_mins,
            "target_mins": h.settings.goal_target_mins,
        },
        "streak": { "current": current, "best": best },
    })))
}
