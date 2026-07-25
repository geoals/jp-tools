//! `/api/sessions` — the sittings a day was made of.
//!
//! Two kinds side by side and deliberately not merged: sessions *derived* from
//! the line stream, and sessions *entered by hand* for reading a texthooker
//! can't see.

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::clock::now_ts;
use crate::db;
use crate::error::AppError;
use crate::history::History;
use crate::stats;

/// How far either side of a day to look when deriving its sessions, so a
/// session straddling the rollover derives against its real neighbours.
const PAD_SECS: f64 = 21600.0;

#[derive(Deserialize)]
pub struct SessionsParams {
    date: Option<String>,
}

pub async fn list_sessions(
    State(state): State<AppState>,
    Query(params): Query<SessionsParams>,
) -> Result<Json<Value>, AppError> {
    let h = History::load(&state).await?;
    let date = match params.date {
        Some(s) => s
            .parse::<NaiveDate>()
            .map_err(|_| AppError::BadRequest(format!("bad date: {s}")))?,
        None => h.today,
    };

    let day_start = h.day_start(date);
    let day_end = day_start + 86400.0;
    // Derive over a padded window, then keep only sessions that *start* on the
    // requested day.
    let lines = h.lines_in(day_start - PAD_SECS, day_end + PAD_SECS);
    let derived: Vec<_> = stats::derive_sessions(lines, &h.presence(), h.settings.session_gap_secs)
        .into_iter()
        .filter(|s| h.date_of(s.start_ts) == date)
        .collect();

    // Cards mined during each session's timespan (note id = creation ms).
    let with_cards = |start: f64, end: f64, v: Value| {
        let mut v = v;
        v["cards"] = json!(h.cards_in(start, end));
        v
    };
    let derived: Vec<Value> = derived
        .into_iter()
        .map(|s| {
            let (start, end) = (s.start_ts, s.end_ts);
            with_cards(start, end, serde_json::to_value(s).unwrap())
        })
        .collect();
    let manual: Vec<Value> = db::fetch_sessions(&state.pool, day_start, day_end)
        .await?
        .into_iter()
        .map(|s| {
            let (start, end) = (s.start_ts, s.end_ts);
            with_cards(start, end, serde_json::to_value(s).unwrap())
        })
        .collect();

    Ok(Json(json!({
        "date": date.to_string(),
        "derived": derived,
        "manual": manual,
    })))
}

#[derive(Deserialize)]
pub struct CreateSession {
    /// Day the session belongs to (defaults to today); ignored when start_ts given.
    pub date: Option<String>,
    pub start_ts: Option<f64>,
    pub minutes: f64,
    /// Exact character count; when absent, pages × chars_per_page is used.
    pub chars: Option<i64>,
    pub pages: Option<f64>,
    pub work: Option<String>,
    pub source: Option<String>,
    pub note: Option<String>,
}

pub async fn create_session(
    State(state): State<AppState>,
    Json(req): Json<CreateSession>,
) -> Result<Json<db::ManualSession>, AppError> {
    if !(req.minutes > 0.0) {
        return Err(AppError::BadRequest("minutes must be > 0".into()));
    }
    let settings = db::load_settings(&state.pool).await?;
    let tz = crate::clock::tz_offset_secs();

    let chars = match (req.chars, req.pages) {
        (Some(c), _) if c >= 0 => c,
        (None, Some(p)) if p > 0.0 => (p * settings.chars_per_page).round() as i64,
        _ => return Err(AppError::BadRequest("need chars or pages".into())),
    };

    let start_ts = match (req.start_ts, &req.date) {
        (Some(ts), _) => ts,
        (None, Some(d)) => {
            let date = d
                .parse::<NaiveDate>()
                .map_err(|_| AppError::BadRequest(format!("bad date: {d}")))?;
            // mid-day anchor: rollover hour + 8h (12:00 local at the default 04)
            stats::day_start_ts(date, settings.day_rollover_hour, tz) + 8.0 * 3600.0
        }
        (None, None) => now_ts() - req.minutes * 60.0,
    };

    let session = db::insert_session(
        &state.pool,
        start_ts,
        start_ts + req.minutes * 60.0,
        chars,
        req.source.as_deref().unwrap_or("book"),
        req.work.as_deref(),
        req.pages,
        req.note.as_deref(),
    )
    .await?;
    Ok(Json(session))
}

pub async fn delete_session(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    if !db::delete_session(&state.pool, id).await? {
        return Err(AppError::NotFound);
    }
    Ok(Json(json!({ "deleted": id })))
}
