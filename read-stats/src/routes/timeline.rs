//! `/api/day/timeline` — one day's intra-day reading curve.

use axum::Json;
use axum::extract::{Query, State};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::AppError;
use crate::history::History;
use crate::stats;

/// How far either side of a day to look, so a session straddling the rollover
/// derives against its real neighbours. Also applied to the lookups: a gap is
/// labelled a lookup gap by the lookups inside it, so a lookup just before the
/// boundary still has to be visible to classify the gap it sits in.
const PAD_SECS: f64 = 21600.0;

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
    let h = History::load(&state).await?;
    let date = match params.date {
        Some(s) => s
            .parse::<NaiveDate>()
            .map_err(|_| AppError::BadRequest(format!("bad date: {s}")))?,
        None => h.today,
    };
    // 15s floor: below that a bucket rarely holds a whole line and the curve is
    // quantization noise. 1h ceiling: past that it isn't a day curve any more —
    // and never above the session gap, or two sessions could share a bucket
    // index and `add_events` would have no way to tell them apart.
    let bucket_ceiling = 3600.0_f64.min(h.settings.session_gap_secs).max(15.0);
    let bucket_secs = params
        .bucket_secs
        .unwrap_or(60.0)
        .clamp(15.0, bucket_ceiling);

    let day_start = h.day_start(date);
    let day_end = day_start + 86400.0;

    let lines = h.lines_in(day_start - PAD_SECS, day_end + PAD_SECS);
    let lookups = h.lookups_in(day_start - PAD_SECS, day_end + PAD_SECS);
    let presence = h.presence();

    let mut buckets = stats::bucket_lines(
        lines,
        lookups,
        &presence,
        h.settings.session_gap_secs,
        bucket_secs,
    );
    buckets.retain(|b| b.t >= day_start && b.t < day_end);

    stats::add_events(&mut buckets, lookups, bucket_secs, stats::EventKind::Lookup);
    let cards = h.card_times_in(day_start, day_end);
    stats::add_events(&mut buckets, &cards, bucket_secs, stats::EventKind::Card);

    // Session spans, so the client can label the bands it draws between.
    let sessions: Vec<Value> =
        stats::derive_sessions(lines, &presence, h.settings.session_gap_secs)
            .into_iter()
            .filter(|s| h.date_of(s.start_ts) == date)
            .map(|s| {
                let (start, end) = (s.start_ts, s.end_ts);
                json!({
                    "start_ts": start,
                    "end_ts": end,
                    "chars": s.chars,
                    "active_secs": s.active_secs,
                    "lines": s.lines,
                    "lookups": lookups.iter().filter(|ts| **ts >= start && **ts <= end).count(),
                    "cards": h.cards_in(start, end),
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
