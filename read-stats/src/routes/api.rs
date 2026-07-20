use std::collections::BTreeMap;
use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, Path, Query, State};
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

/// Sorted proofs that the reader was at the keyboard over `[from, to)`:
/// dictionary lookups and mined cards, with paused spans excluded the same way
/// the lines are. `stats::Presence` credits gap time against these.
async fn presence_marks(
    state: &AppState,
    pauses: &[stats::PauseInterval],
    from: f64,
    to: f64,
) -> Result<Vec<f64>, AppError> {
    let lookups = db::fetch_lookup_events(&state.pool, from, to).await?;
    // Note ids are epoch milliseconds, so they double as card creation times.
    let cards: Vec<f64> = db::fetch_anki_note_ids(&state.pool)
        .await?
        .iter()
        .map(|id| *id as f64 / 1000.0)
        .filter(|ts| *ts >= from && *ts < to)
        .collect();
    let mut marks = stats::presence_marks(&lookups, &cards);
    marks.retain(|ts| !stats::is_paused(*ts, pauses));
    Ok(marks)
}

/// The one window every endpoint prices absence against: all of history.
///
/// Pace is a property of the reader, not of the slice a request happened to
/// fetch. Deriving it per-request had the dashboard measuring it over all days
/// and the timeline over one, so the same day's active minutes differed
/// depending on which page you were looking at.
async fn reader_pace(
    state: &AppState,
    settings: &Settings,
    pauses: &[stats::PauseInterval],
) -> Result<Option<f64>, AppError> {
    let mut lines = db::fetch_line_events(&state.pool, 0.0, f64::MAX).await?;
    lines.retain(|l| !stats::is_paused(l.ts, pauses));
    let marks = presence_marks(state, pauses, 0.0, f64::MAX).await?;
    Ok(stats::measure_pace(&lines, &marks, settings.afk_secs))
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
    let marks = presence_marks(state, &pauses, 0.0, f64::MAX).await?;
    let pace = stats::measure_pace(&lines, &marks, settings.afk_secs);
    let presence = stats::Presence::new(&marks, pace, settings.afk_secs);
    let vn = stats::aggregate_line_days(
        &lines,
        &presence,
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

/// Per-day focus (how continuous the reading was) from the raw line stream.
async fn focus_days(
    state: &AppState,
    settings: &Settings,
    tz: i64,
) -> Result<BTreeMap<NaiveDate, stats::FocusDay>, AppError> {
    let pauses = db::fetch_pauses(&state.pool).await?;
    let mut lines = db::fetch_line_events(&state.pool, 0.0, f64::MAX).await?;
    lines.retain(|l| !stats::is_paused(l.ts, &pauses));
    let marks = presence_marks(state, &pauses, 0.0, f64::MAX).await?;
    let pace = stats::measure_pace(&lines, &marks, settings.afk_secs);
    let presence = stats::Presence::new(&marks, pace, settings.afk_secs);
    Ok(stats::aggregate_focus_days(
        &lines,
        &presence,
        settings.session_gap_secs,
        settings.day_rollover_hour,
        tz,
    ))
}

fn focus_json(f: &stats::FocusDay) -> Value {
    json!({
        "ratio": f.ratio(),
        "span_secs": f.span_secs,
        "longest_stretch_secs": f.longest_stretch_secs,
        "interruptions": f.interruptions,
    })
}

/// Yomitan lookups per day. Paused intervals are dropped for the same reason
/// lines are: re-reading skipped text shouldn't move the numbers.
async fn lookup_days(
    state: &AppState,
    settings: &Settings,
    tz: i64,
) -> Result<BTreeMap<NaiveDate, i64>, AppError> {
    let pauses = db::fetch_pauses(&state.pool).await?;
    let mut out: BTreeMap<NaiveDate, i64> = BTreeMap::new();
    for ts in db::fetch_lookup_events(&state.pool, 0.0, f64::MAX).await? {
        if !stats::is_paused(ts, &pauses) {
            *out.entry(stats::date_key(ts, settings.day_rollover_hour, tz))
                .or_default() += 1;
        }
    }
    Ok(out)
}

/// Lookups per 1000 characters — the unknown-word rate, and the number that
/// says whether a work sits at the comprehension edge. None below a floor of
/// chars, where the ratio is dominated by noise.
fn lookup_rate(lookups: i64, chars: i64) -> Option<f64> {
    (chars >= 500).then(|| lookups as f64 * 1000.0 / chars as f64)
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

    let day_start = stats::day_start_ts(today, settings.day_rollover_hour, tz);
    let note_ids = db::fetch_anki_note_ids(&state.pool).await?;
    let today_lookups = lookup_days(&state, &settings, tz)
        .await?
        .get(&today)
        .copied()
        .unwrap_or(0);

    Ok(Json(json!({
        "paused": db::is_pause_open(&state.pool).await?,
        "today": {
            "date": today.to_string(),
            "chars": today_total.chars,
            "active_secs": today_total.active_secs,
            "vn": today_vn,
            "manual": today_manual,
            "cards": cards_in_window(&note_ids, day_start, day_start + 86400.0),
            "lookups": today_lookups,
            "lookups_per_1k": lookup_rate(today_lookups, today_total.chars),
            "focus": focus_json(
                &focus_days(&state, &settings, tz).await?.get(&today).copied().unwrap_or_default(),
            ),
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
    let lookups = lookup_days(&state, &settings, tz).await?;
    let focus = focus_days(&state, &settings, tz).await?;
    let note_ids = db::fetch_anki_note_ids(&state.pool).await?;

    // Zero-filled, oldest → newest, so charts never have to infer missing days.
    let mut out = Vec::with_capacity(n as usize);
    for i in (0..n).rev() {
        let date = today - chrono::Duration::days(i);
        let v = vn.get(&date).copied().unwrap_or_default();
        let m = manual.get(&date).copied().unwrap_or_default();
        let l = lookups.get(&date).copied().unwrap_or(0);
        let day_start = stats::day_start_ts(date, settings.day_rollover_hour, tz);
        out.push(json!({
            "date": date.to_string(),
            "chars": v.chars + m.chars,
            "active_secs": v.active_secs + m.active_secs,
            "vn": v,
            "manual": m,
            "lookups": l,
            "lookups_per_1k": lookup_rate(l, v.chars + m.chars),
            "cards": cards_in_window(&note_ids, day_start, day_start + 86400.0),
            "focus": focus_json(&focus.get(&date).copied().unwrap_or_default()),
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
    let marks = presence_marks(&state, &pauses, day_start - 21600.0, day_end + 21600.0).await?;
    // Pace over all history, not this day's slice, so the dashboard and the
    // timeline price the same day's absence identically.
    let presence = stats::Presence::new(
        &marks,
        reader_pace(&state, &settings, &pauses).await?,
        settings.afk_secs,
    );
    let derived: Vec<_> = stats::derive_sessions(&lines, &presence, settings.session_gap_secs)
        .into_iter()
        .filter(|s| stats::date_key(s.start_ts, settings.day_rollover_hour, tz) == date)
        .collect();

    let manual: Vec<_> = db::fetch_sessions(&state.pool, day_start, day_end).await?;

    // Cards mined during each session's timespan (note id = creation ms).
    let note_ids = db::fetch_anki_note_ids(&state.pool).await?;
    let with_cards = |start: f64, end: f64, v: Value| {
        let mut v = v;
        v["cards"] = json!(cards_in_window(&note_ids, start, end));
        v
    };
    let derived: Vec<Value> = derived
        .into_iter()
        .map(|s| {
            let (start, end) = (s.start_ts, s.end_ts);
            with_cards(start, end, serde_json::to_value(s).unwrap())
        })
        .collect();
    let manual: Vec<Value> = manual
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
pub struct TimelineParams {
    date: Option<String>,
    bucket_secs: Option<f64>,
}

/// Intra-day reading curve: fine-grained buckets of chars, active time,
/// lookups and mined cards for one day.
///
/// The buckets are deliberately finer than anything worth plotting (one minute
/// by default). Smoothing is the client's job, so dragging the granularity
/// control is instant and never re-queries — which also means it can't perturb
/// a reading session that's still in progress.
pub async fn day_timeline(
    State(state): State<AppState>,
    Query(params): Query<TimelineParams>,
) -> Result<Json<Value>, AppError> {
    let settings = db::load_settings(&state.pool).await?;
    let tz = tz_offset_secs();
    let date = match params.date {
        Some(s) => s
            .parse::<NaiveDate>()
            .map_err(|_| AppError::BadRequest(format!("bad date: {s}")))?,
        None => stats::date_key(now_ts(), settings.day_rollover_hour, tz),
    };
    // 15s floor: below that a bucket rarely holds a whole line and the curve is
    // quantization noise. 1h ceiling: past that it isn't a day curve any more —
    // and never above the session gap, or two sessions could share a bucket
    // index and `add_events` would have no way to tell them apart.
    let bucket_ceiling = 3600.0_f64.min(settings.session_gap_secs).max(15.0);
    let bucket_secs = params.bucket_secs.unwrap_or(60.0).clamp(15.0, bucket_ceiling);

    let day_start = stats::day_start_ts(date, settings.day_rollover_hour, tz);
    let day_end = day_start + 86400.0;

    // Pad the fetch so a session straddling the rollover derives with its real
    // neighbours, then keep only the buckets belonging to the requested day.
    let pauses = db::fetch_pauses(&state.pool).await?;
    let mut lines =
        db::fetch_line_events(&state.pool, day_start - 21600.0, day_end + 21600.0).await?;
    lines.retain(|l| !stats::is_paused(l.ts, &pauses));

    // Fetched over the same padded window as the lines: a gap is labelled a
    // lookup gap by the lookups inside it, so a lookup just before the day
    // boundary still has to be visible to classify the gap it sits in.
    let mut lookups: Vec<f64> =
        db::fetch_lookup_events(&state.pool, day_start - 21600.0, day_end + 21600.0).await?;
    lookups.retain(|ts| !stats::is_paused(*ts, &pauses));

    let marks = presence_marks(&state, &pauses, day_start - 21600.0, day_end + 21600.0).await?;
    // Pace over all history, not this day's slice, so the dashboard and the
    // timeline price the same day's absence identically.
    let presence = stats::Presence::new(
        &marks,
        reader_pace(&state, &settings, &pauses).await?,
        settings.afk_secs,
    );

    let mut buckets = stats::bucket_lines(
        &lines,
        &lookups,
        &presence,
        settings.session_gap_secs,
        bucket_secs,
    );
    buckets.retain(|b| b.t >= day_start && b.t < day_end);

    stats::add_events(&mut buckets, &lookups, bucket_secs, stats::EventKind::Lookup);

    // Note ids are epoch milliseconds, so they double as card creation times.
    let note_ids = db::fetch_anki_note_ids(&state.pool).await?;
    let cards: Vec<f64> = note_ids
        .iter()
        .map(|id| *id as f64 / 1000.0)
        .filter(|ts| *ts >= day_start && *ts < day_end)
        .collect();
    stats::add_events(&mut buckets, &cards, bucket_secs, stats::EventKind::Card);

    // Session spans, so the client can label the bands it draws between.
    let sessions: Vec<Value> = stats::derive_sessions(&lines, &presence, settings.session_gap_secs)
        .into_iter()
    .filter(|s| stats::date_key(s.start_ts, settings.day_rollover_hour, tz) == date)
    .map(|s| {
        let (start, end) = (s.start_ts, s.end_ts);
        json!({
            "start_ts": start,
            "end_ts": end,
            "chars": s.chars,
            "active_secs": s.active_secs,
            "lines": s.lines,
            "lookups": lookups.iter().filter(|ts| **ts >= start && **ts <= end).count(),
            "cards": cards_in_window(&note_ids, start, end),
        })
    })
    .collect();

    Ok(Json(json!({
        "date": date.to_string(),
        "bucket_secs": bucket_secs,
        "day_start": day_start,
        "sessions": sessions,
        "buckets": buckets,
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

fn meta_json(m: &db::Work) -> Value {
    json!({
        "id": m.id,
        "total_chars": m.total_chars,
        "cover": m.cover_path.as_ref().map(|p| format!("/covers/{p}")),
        "status": m.status,
        "queue_pos": m.queue_pos,
    })
}

#[derive(Deserialize)]
pub struct WorkMetaReq {
    /// Exact title as stamped on lines/sessions — the join key. Required on POST.
    pub title: Option<String>,
    /// "v3144", "3144", or a vndb.org URL — used once to fetch the cover,
    /// never stored. Empty string removes the cover.
    pub vndb_id: Option<String>,
    /// Pasted from jpdb. 0 clears it.
    pub total_chars: Option<i64>,
    pub status: Option<String>,
    pub queue_pos: Option<i64>,
}

/// Apply the optional fields of a metadata request to an existing work row,
/// doing the one-shot VNDB cover fetch when a vndb id is given.
async fn apply_work_meta(
    state: &AppState,
    id: i64,
    req: &WorkMetaReq,
) -> Result<(), AppError> {
    if let Some(raw) = &req.vndb_id {
        let old_cover = db::fetch_work(&state.pool, id)
            .await?
            .and_then(|w| w.cover_path);
        let new_cover = if raw.trim().is_empty() {
            None
        } else {
            let vid = crate::vndb::normalize_id(raw)
                .ok_or_else(|| AppError::BadRequest(format!("bad vndb id: {raw}")))?;
            let url = crate::vndb::fetch_cover_url(&state.http, &vid).await?;
            Some(
                crate::vndb::download_cover(&state.http, &url, &state.covers_dir, &format!("w{id}"))
                    .await?,
            )
        };
        db::set_work_cover(&state.pool, id, new_cover.as_deref()).await?;
        if let Some(old) = old_cover.filter(|old| Some(old) != new_cover.as_ref()) {
            let _ = tokio::fs::remove_file(state.covers_dir.join(&old)).await;
        }
    }
    if let Some(total) = req.total_chars {
        if total < 0 {
            return Err(AppError::BadRequest("total_chars must be >= 0".into()));
        }
        db::set_work_total_chars(&state.pool, id, (total > 0).then_some(total)).await?;
    }
    if let Some(status) = &req.status {
        if !db::WORK_STATUSES.contains(&status.as_str()) {
            return Err(AppError::BadRequest(format!(
                "status must be one of {:?}",
                db::WORK_STATUSES
            )));
        }
        db::set_work_status(&state.pool, id, status).await?;
    }
    if let Some(pos) = req.queue_pos {
        db::set_work_queue_pos(&state.pool, id, (pos >= 0).then_some(pos)).await?;
    }
    Ok(())
}

/// Create-or-update work metadata, keyed by exact title.
pub async fn upsert_work(
    State(state): State<AppState>,
    Json(req): Json<WorkMetaReq>,
) -> Result<Json<db::Work>, AppError> {
    let title = req
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| AppError::BadRequest("title required".into()))?;
    let work = db::upsert_work(&state.pool, title).await?;
    apply_work_meta(&state, work.id, &req).await?;
    Ok(Json(db::fetch_work(&state.pool, work.id).await?.ok_or(AppError::NotFound)?))
}

pub async fn update_work(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<WorkMetaReq>,
) -> Result<Json<db::Work>, AppError> {
    db::fetch_work(&state.pool, id).await?.ok_or(AppError::NotFound)?;
    apply_work_meta(&state, id, &req).await?;
    Ok(Json(db::fetch_work(&state.pool, id).await?.ok_or(AppError::NotFound)?))
}

pub async fn delete_work(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    let work = db::fetch_work(&state.pool, id).await?.ok_or(AppError::NotFound)?;
    if let Some(cover) = &work.cover_path {
        let _ = tokio::fs::remove_file(state.covers_dir.join(cover)).await;
    }
    db::delete_work(&state.pool, id).await?;
    Ok(Json(json!({ "deleted": id })))
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

    // Metadata joins by exact title; leftovers (queued works with no lines
    // yet) still get a row so they show up before reading starts.
    let mut meta_by_title: BTreeMap<String, db::Work> = db::fetch_works_meta(&state.pool)
        .await?
        .into_iter()
        .map(|w| (w.title.clone(), w))
        .collect();

    let tz = tz_offset_secs();
    let mut list: Vec<_> = agg
        .into_iter()
        .map(|(work, a)| {
            let meta = work.as_ref().and_then(|t| meta_by_title.remove(t));
            json!({
                "work": work,
                "chars": a.chars,
                "active_secs": a.active_secs,
                "first_read": stats::date_key(a.first_ts, settings.day_rollover_hour, tz).to_string(),
                "last_read": stats::date_key(a.last_ts, settings.day_rollover_hour, tz).to_string(),
                "meta": meta.as_ref().map(meta_json),
            })
        })
        .collect();
    for (title, m) in meta_by_title {
        list.push(json!({
            "work": title,
            "chars": 0,
            "active_secs": 0.0,
            "first_read": null,
            "last_read": null,
            "meta": meta_json(&m),
        }));
    }
    list.sort_by(|a, b| {
        b["last_read"]
            .as_str()
            .cmp(&a["last_read"].as_str())
            .then(b["chars"].as_i64().cmp(&a["chars"].as_i64()))
    });
    Ok(Json(json!(list)))
}

/// Count note ids (epoch ms) falling inside a [start, end) seconds window.
/// How many lookups turn into cards, and which ones didn't work out.
///
/// Three outcomes per distinct term, decided by comparing the card's creation
/// time (the note id, epoch ms) against the term's first lookup:
///   - **mined** — a card was made at or after the lookup: the lookup stuck.
///   - **known** — a card already existed: a word that was mined but didn't
///     take, i.e. a leech worth reformulating.
///   - **unmined** — looked up, never carded. Repeats here are mining
///     candidates: the same word slowed you down more than once.
///
/// Counts are over *distinct terms*, not lookup events, so a word looked up
/// five times before being mined counts once and can't inflate the rate.
pub async fn lookups_summary(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    const LIST_CAP: usize = 12;
    let terms = db::fetch_lookup_terms(&state.pool).await?;

    let (mut mined, mut known, mut unmined) = (0i64, 0i64, 0i64);
    let mut leeches: Vec<&db::LookupTerm> = Vec::new();
    // Lookup → card latency, a first read on what mining actually costs.
    let mut mine_lags: Vec<f64> = Vec::new();

    let status_of = |t: &db::LookupTerm| match t.note_id {
        Some(id) if id as f64 / 1000.0 >= t.first_ts => "mined",
        Some(_) => "known",
        None => "unmined",
    };

    for t in &terms {
        match status_of(t) {
            "mined" => {
                mined += 1;
                // From the lookup that led to the card, not the first ever.
                if let Some(from) = t.mine_from_ts {
                    mine_lags.push(t.note_id.unwrap() as f64 / 1000.0 - from);
                }
            }
            "known" => {
                known += 1;
                leeches.push(t);
            }
            _ => unmined += 1,
        }
    }

    // Words looked up more than once, worst first — the ones costing repeat
    // time. Status rides along: an unmined repeat is a mining candidate, a
    // known repeat is a card that isn't working.
    let mut repeats: Vec<&db::LookupTerm> = terms.iter().filter(|t| t.times > 1).collect();

    // Worst first: most re-looked-up, then most recent.
    let by_weight = |a: &&db::LookupTerm, b: &&db::LookupTerm| {
        b.times
            .cmp(&a.times)
            .then(b.last_ts.total_cmp(&a.last_ts))
    };
    leeches.sort_by(by_weight);
    repeats.sort_by(by_weight);

    mine_lags.sort_by(f64::total_cmp);
    let median_mine_secs = (!mine_lags.is_empty()).then(|| mine_lags[mine_lags.len() / 2]);

    let brief = |list: &[&db::LookupTerm]| -> Vec<Value> {
        list.iter()
            .take(LIST_CAP)
            .map(|t| {
                json!({
                    "term": t.term,
                    "times": t.times,
                    "last_ts": t.last_ts,
                    "status": status_of(t),
                    // Days since the card was made — a long-standing card still
                    // being looked up is the strongest leech signal.
                    "card_age_days": t.note_id.map(|id| {
                        ((now_ts() - id as f64 / 1000.0) / 86400.0).floor()
                    }),
                })
            })
            .collect()
    };

    Ok(Json(json!({
        "terms": terms.len(),
        "events": terms.iter().map(|t| t.times).sum::<i64>(),
        "mined": mined,
        "known": known,
        "unmined": unmined,
        "median_mine_secs": median_mine_secs,
        "repeat_terms": repeats.len(),
        // Lookups spent re-reading a word already looked up before.
        "repeat_events": repeats.iter().map(|t| t.times - 1).sum::<i64>(),
        "repeats": brief(&repeats),
        "leeches": brief(&leeches),
        "leech_count": leeches.len(),
    })))
}

fn cards_in_window(note_ids: &[i64], start_ts: f64, end_ts: f64) -> i64 {
    let (a, b) = ((start_ts * 1000.0) as i64, (end_ts * 1000.0) as i64);
    let lo = note_ids.partition_point(|&id| id < a);
    let hi = note_ids.partition_point(|&id| id < b);
    (hi - lo) as i64
}

/// Probe for AnkiConnect (dashboard client first, then the configured
/// fallback), snapshot the mined deck, then tokenize any new lines.
pub async fn anki_refresh(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Result<Json<Value>, AppError> {
    let mut last_err = AppError::Upstream(format!(
        "no AnkiConnect reachable (tried dashboard client {} and {})",
        addr.ip(),
        state.anki_url
    ));
    let mut snapshot = None;
    for url in crate::anki::candidate_urls(Some(addr.ip()), &state.anki_url) {
        match crate::anki::fetch_deck_vocab(&state.http, &url, &state.anki_deck, &state.anki_vocab_field)
            .await
        {
            Ok(notes) => {
                snapshot = Some((url, notes));
                break;
            }
            Err(e) => last_err = e,
        }
    }
    let Some((source, notes)) = snapshot else {
        return Err(last_err);
    };

    db::replace_anki_notes(&state.pool, &notes).await?;
    db::save_setting(&state.pool, "anki_snapshot_ts", &now_ts().to_string()).await?;
    db::save_setting(&state.pool, "anki_source", &source).await?;
    let ingest = crate::ingest::ingest_new_lines(&state).await?;

    Ok(Json(json!({ "notes": notes.len(), "source": source, "ingest": ingest })))
}

/// Re-encounter statistics: how often mined words reappear in the line stream.
pub async fn anki_summary(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let Some(snapshot_ts) = db::get_setting_raw(&state.pool, "anki_snapshot_ts")
        .await?
        .and_then(|v| v.parse::<f64>().ok())
    else {
        return Ok(Json(json!({ "available": false })));
    };

    let settings = db::load_settings(&state.pool).await?;
    let tz = tz_offset_secs();
    let rollover = settings.day_rollover_hour;
    let today = stats::date_key(now_ts(), rollover, tz);
    let week_start = (today - chrono::Duration::days(6)).to_string();

    // Earliest note per vocab (dupes possible when a word was re-mined).
    let mut mined: BTreeMap<String, (i64, String)> = BTreeMap::new();
    for n in db::fetch_anki_notes(&state.pool).await? {
        let date = stats::date_key(n.note_id as f64 / 1000.0, rollover, tz).to_string();
        mined.entry(n.vocab).or_insert((n.note_id, date));
    }

    // Encounters per mined lemma, split into after-mined-day and last-7-days.
    let mut after: BTreeMap<&str, i64> = BTreeMap::new();
    let mut week: BTreeMap<&str, i64> = BTreeMap::new();
    let hits = db::fetch_mined_word_days(&state.pool).await?;
    for h in &hits {
        let Some((_, mined_date)) = mined.get(&h.lemma) else { continue };
        if h.date > *mined_date {
            *after.entry(h.lemma.as_str()).or_default() += h.count;
        }
        if h.date >= week_start {
            *week.entry(h.lemma.as_str()).or_default() += h.count;
        }
    }

    let reencountered = after.len() as i64;
    let week_total: i64 = week.values().sum();
    let mut top_week: Vec<_> = week.iter().map(|(w, c)| (*w, *c)).collect();
    top_week.sort_by(|a, b| b.1.cmp(&a.1));
    let top_week: Vec<Value> = top_week
        .iter()
        .take(10)
        .map(|(w, c)| json!({ "word": w, "count": c }))
        .collect();

    // Never re-encountered since mined, oldest cards first.
    let mut never: Vec<_> = mined
        .iter()
        .filter(|(vocab, _)| !after.contains_key(vocab.as_str()))
        .map(|(vocab, (note_id, _))| (*note_id, vocab.clone()))
        .collect();
    never.sort();

    Ok(Json(json!({
        "available": true,
        "snapshot_ts": snapshot_ts,
        "source": db::get_setting_raw(&state.pool, "anki_source").await?,
        "mined": mined.len(),
        "reencountered": reencountered,
        "week_encounters": week_total,
        "top_week": top_week,
        "never_count": never.len(),
        "never_sample": never.iter().take(10).map(|(_, w)| w).collect::<Vec<_>>(),
    })))
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
        let stored = if key == "current_work" || key == "pace_start_date" || key == "vn_window" {
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
