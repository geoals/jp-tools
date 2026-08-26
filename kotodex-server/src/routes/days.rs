//! `/api/days` — the daily series behind every chart on the dashboard.

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::AppError;
use crate::history::History;
use crate::routes::json::focus_json;
use crate::stats;

#[derive(Deserialize)]
pub struct DaysParams {
    days: Option<i64>,
}

pub async fn days(
    State(state): State<AppState>,
    Query(params): Query<DaysParams>,
) -> Result<Json<Value>, AppError> {
    let n = params.days.unwrap_or(30).clamp(1, 3650);
    let h = History::load(&state).await?;

    let (vn, manual) = h.day_maps();
    let lookups = h.lookup_days();
    let measured = h.measured_days();
    let clean = h.clean_days();
    let focus = h.focus_days();
    let work_days = h.dominant_work_days();

    // Zero-filled, oldest → newest, so charts never have to infer missing days.
    let mut out = Vec::with_capacity(n as usize);
    for i in (0..n).rev() {
        let date = h.today - chrono::Duration::days(i);
        let v = vn.get(&date).copied().unwrap_or_default();
        let m = manual.get(&date).copied().unwrap_or_default();
        let l = lookups.get(&date).copied().unwrap_or(0);
        let day_start = h.day_start(date);
        out.push(json!({
            "date": date.to_string(),
            "chars": v.chars + m.chars,
            "active_secs": v.active_secs + m.active_secs,
            "vn": v,
            "manual": m,
            // Reading whose duration was measured rather than derived — the
            // only honest denominator for chars/hour. See measured_days.
            "measured": measured.get(&date).copied().unwrap_or_default(),
            // The same measure with the lookups taken out — the pace on the
            // text itself. See History::clean_days.
            "clean": clean.get(&date).copied().unwrap_or_default(),
            "lookups": l,
            // Hooked characters only. Lookups are recorded only while the
            // line stream is live, so manual characters can enter this
            // denominator but never its numerator. See stats::rate.
            "lookups_per_1k": stats::rate::lookups_per_1k(l, v.chars),
            "cards": h.cards_in(day_start, day_start + 86400.0),
            "focus": focus_json(&focus.get(&date).copied().unwrap_or_default()),
            "work": work_days.get(&date),
        }));
    }
    Ok(Json(json!(out)))
}
