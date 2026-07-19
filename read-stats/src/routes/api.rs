use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{Local, NaiveDate};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::db::{self, Settings};
use crate::error::AppError;
use crate::stats::{self, DayBucket};

fn tz_offset_secs() -> i64 {
    Local::now().offset().local_minus_utc() as i64
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// Per-day totals for both sources over the whole history.
async fn day_maps(
    state: &AppState,
    settings: &Settings,
    tz: i64,
) -> Result<
    (
        BTreeMap<NaiveDate, DayBucket>,
        BTreeMap<NaiveDate, DayBucket>,
    ),
    AppError,
> {
    let pauses = db::fetch_pauses(&state.pool).await?;
    let mut lines = db::fetch_line_events(&state.pool, 0.0, f64::MAX).await?;
    lines.retain(|l| !stats::is_paused(l.ts, &pauses));
    let vn = stats::aggregate_line_days(
        &lines,
        settings.afk_secs,
        settings.session_gap_secs,
        settings.day_rollover_hour,
        tz,
    );

    let mut manual: BTreeMap<NaiveDate, DayBucket> = BTreeMap::new();
    for s in db::fetch_sessions(&state.pool, 0.0, f64::MAX).await? {
        let day = manual
            .entry(stats::date_key(s.start_ts, settings.day_rollover_hour, tz))
            .or_default();
        day.chars += s.chars;
        day.active_secs += (s.end_ts - s.start_ts).max(0.0);
    }
    Ok((vn, manual))
}

fn merged(
    vn: &BTreeMap<NaiveDate, DayBucket>,
    manual: &BTreeMap<NaiveDate, DayBucket>,
) -> BTreeMap<NaiveDate, DayBucket> {
    let mut out = vn.clone();
    for (date, bucket) in manual {
        let day = out.entry(*date).or_default();
        day.chars += bucket.chars;
        day.active_secs += bucket.active_secs;
    }
    out
}

pub async fn summary(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let settings = db::load_settings(&state.pool).await?;
    let tz = tz_offset_secs();
    let today = stats::date_key(now_ts(), settings.day_rollover_hour, tz);

    let (vn, manual) = day_maps(&state, &settings, tz).await?;
    let total = merged(&vn, &manual);

    let today_total = total.get(&today).copied().unwrap_or_default();
    let today_vn = vn.get(&today).copied().unwrap_or_default();
    let today_manual = manual.get(&today).copied().unwrap_or_default();

    let (current, best) = stats::streaks(
        &total,
        settings.goal_floor_mins as f64 * 60.0,
        today,
    );

    Ok(Json(json!({
        "paused": db::is_pause_open(&state.pool).await?,
        "today": {
            "date": today.to_string(),
            "chars": today_total.chars,
            "active_secs": today_total.active_secs,
            "vn": today_vn,
            "manual": today_manual,
        },
        "goal": {
            "floor_mins": settings.goal_floor_mins,
            "target_mins": settings.goal_target_mins,
        },
        "streak": { "current": current, "best": best },
    })))
}

#[derive(Deserialize)]
pub struct DaysParams {
    days: Option<i64>,
}

pub async fn days(
    State(state): State<AppState>,
    Query(params): Query<DaysParams>,
) -> Result<Json<Value>, AppError> {
    let n = params.days.unwrap_or(30).clamp(1, 3650);
    let settings = db::load_settings(&state.pool).await?;
    let tz = tz_offset_secs();
    let today = stats::date_key(now_ts(), settings.day_rollover_hour, tz);

    let (vn, manual) = day_maps(&state, &settings, tz).await?;

    // Zero-filled, oldest → newest, so charts never have to infer missing days.
    let mut out = Vec::with_capacity(n as usize);
    for i in (0..n).rev() {
        let date = today - chrono::Duration::days(i);
        let v = vn.get(&date).copied().unwrap_or_default();
        let m = manual.get(&date).copied().unwrap_or_default();
        out.push(json!({
            "date": date.to_string(),
            "chars": v.chars + m.chars,
            "active_secs": v.active_secs + m.active_secs,
            "vn": v,
            "manual": m,
        }));
    }
    Ok(Json(json!(out)))
}

#[derive(Deserialize)]
pub struct SessionsParams {
    date: Option<String>,
}

pub async fn list_sessions(
    State(state): State<AppState>,
    Query(params): Query<SessionsParams>,
) -> Result<Json<Value>, AppError> {
    let settings = db::load_settings(&state.pool).await?;
    let tz = tz_offset_secs();
    let date = match params.date {
        Some(s) => s
            .parse::<NaiveDate>()
            .map_err(|_| AppError::BadRequest(format!("bad date: {s}")))?,
        None => stats::date_key(now_ts(), settings.day_rollover_hour, tz),
    };

    let day_start = stats::day_start_ts(date, settings.day_rollover_hour, tz);
    let day_end = day_start + 86400.0;
    // Pad the window so a session straddling the boundary derives correctly,
    // then keep only sessions that *start* on the requested day.
    let pauses = db::fetch_pauses(&state.pool).await?;
    let mut lines = db::fetch_line_events(&state.pool, day_start - 21600.0, day_end + 21600.0).await?;
    lines.retain(|l| !stats::is_paused(l.ts, &pauses));
    let derived: Vec<_> = stats::derive_sessions(&lines, settings.afk_secs, settings.session_gap_secs)
        .into_iter()
        .filter(|s| stats::date_key(s.start_ts, settings.day_rollover_hour, tz) == date)
        .collect();

    let manual: Vec<_> = db::fetch_sessions(&state.pool, day_start, day_end).await?;

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
    let tz = tz_offset_secs();

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

/// Toggle the tracking pause. Returns `{"paused": bool}`.
pub async fn toggle_pause(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let paused = db::toggle_pause(&state.pool, now_ts()).await?;
    Ok(Json(json!({ "paused": paused })))
}

pub async fn works(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let settings = db::load_settings(&state.pool).await?;
    let pauses = db::fetch_pauses(&state.pool).await?;
    let mut lines = db::fetch_work_lines(&state.pool).await?;
    lines.retain(|l| !stats::is_paused(l.ts, &pauses));
    let mut agg = stats::aggregate_works(&lines, settings.afk_secs, settings.session_gap_secs);

    // Manual sessions merge in by title.
    for s in db::fetch_sessions(&state.pool, 0.0, f64::MAX).await? {
        let entry = agg.entry(s.work.clone()).or_insert_with(|| stats::WorkAgg {
            first_ts: s.start_ts,
            ..Default::default()
        });
        entry.chars += s.chars;
        entry.active_secs += (s.end_ts - s.start_ts).max(0.0);
        entry.first_ts = entry.first_ts.min(s.start_ts);
        entry.last_ts = entry.last_ts.max(s.end_ts);
    }

    let tz = tz_offset_secs();
    let mut list: Vec<_> = agg
        .into_iter()
        .map(|(work, a)| {
            json!({
                "work": work,
                "chars": a.chars,
                "active_secs": a.active_secs,
                "first_read": stats::date_key(a.first_ts, settings.day_rollover_hour, tz).to_string(),
                "last_read": stats::date_key(a.last_ts, settings.day_rollover_hour, tz).to_string(),
            })
        })
        .collect();
    list.sort_by(|a, b| {
        b["last_read"]
            .as_str()
            .cmp(&a["last_read"].as_str())
            .then(b["chars"].as_i64().cmp(&a["chars"].as_i64()))
    });
    Ok(Json(json!(list)))
}

pub async fn get_settings(State(state): State<AppState>) -> Result<Json<Settings>, AppError> {
    Ok(Json(db::load_settings(&state.pool).await?))
}

pub async fn put_settings(
    State(state): State<AppState>,
    Json(updates): Json<serde_json::Map<String, Value>>,
) -> Result<Json<Settings>, AppError> {
    for (key, value) in &updates {
        if !db::SETTING_KEYS.contains(&key.as_str()) {
            return Err(AppError::BadRequest(format!("unknown setting: {key}")));
        }
        let stored = if key == "current_work" {
            let Some(s) = value.as_str() else {
                return Err(AppError::BadRequest("current_work must be a string".into()));
            };
            s.trim().to_string()
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
