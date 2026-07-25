//! `/api/dialogue/summary` — how much of the reading was people talking, and
//! whether speech and prose read at different speeds.
//!
//! Over all history *and* per day: the whole-history figures are the ones with
//! enough characters behind them to compare speeds, while the per-day share
//! tracks what the reading actually consisted of, which swings with the scene.

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::AppError;
use crate::history::History;
use crate::routes::json::side_json;
use crate::stats;

#[derive(Deserialize)]
pub struct DialogueParams {
    days: Option<i64>,
    /// Scope the split to one work's lines. Absent/empty = all works pooled.
    work: Option<String>,
}

pub async fn dialogue_summary(
    State(state): State<AppState>,
    Query(params): Query<DialogueParams>,
) -> Result<Json<Value>, AppError> {
    let n = params.days.unwrap_or(30).clamp(1, 3650);
    let want_work = params
        .work
        .as_deref()
        .map(str::trim)
        .filter(|w| !w.is_empty());
    let h = History::load(&state).await?;

    // Scoping to one VN filters the line stream to its lines before the split
    // is aggregated; lookups outside this work's credited gaps fall out for
    // free. Pace and presence stay whole-history — they are properties of the
    // reader, not of the VN being asked about.
    let lines = match want_work {
        Some(w) => h.lines_of_work(w),
        None => h.lines.clone(),
    };

    let by_day = stats::aggregate_dialogue_days(
        &lines,
        &h.lookups,
        &h.presence(),
        h.settings.session_gap_secs,
        h.settings.day_rollover_hour,
        h.tz,
    );

    let mut overall = stats::DialogueDay::default();
    for day in by_day.values() {
        overall.add(day);
    }

    // Zero-filled like /api/days, but a day with no classified text reports a
    // null share rather than 0% — "no text to split" is not "all narration".
    let mut days = Vec::with_capacity(n as usize);
    for i in (0..n).rev() {
        let date = h.today - chrono::Duration::days(i);
        let d = by_day.get(&date).copied().unwrap_or_default();
        days.push(json!({
            "date": date.to_string(),
            "dialogue_chars": d.dialogue.chars,
            "narration_chars": d.narration.chars,
            // Seconds credited to gaps after lines of each kind. These do not
            // add up to the day's active time and are not meant to: manual
            // sessions have no text, and mixed lines are unattributable. The
            // daily chart stacks them under a remainder segment rather than
            // rescaling, so the shortfall stays visible instead of being
            // silently spread across the two.
            "dialogue_secs": d.dialogue.timed_secs,
            "narration_secs": d.narration.timed_secs,
            "share": d.share(),
        }));
    }

    let today_day = by_day.get(&h.today).copied().unwrap_or_default();
    Ok(Json(json!({
        "today": {
            "dialogue_chars": today_day.dialogue.chars,
            "narration_chars": today_day.narration.chars,
            "share": today_day.share(),
        },
        "overall": {
            "share": overall.share(),
            "dialogue": side_json(&overall.dialogue),
            "narration": side_json(&overall.narration),
        },
        "days": days,
    })))
}
