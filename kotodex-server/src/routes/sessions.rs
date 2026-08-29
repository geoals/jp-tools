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
    // A manual session's duration may be derived rather than recorded, so the
    // span is resolved here and served alongside — the client is never handed
    // a null `end_ts` and left to decide what it means.
    let manual: Vec<Value> = db::fetch_sessions(&state.knowledge, day_start, day_end)
        .await?
        .into_iter()
        .map(|s| {
            let (secs, estimated) = h.duration_of(&s);
            let (start, end) = (s.start_ts, s.start_ts + secs);
            let mut v = with_cards(start, end, serde_json::to_value(s).unwrap());
            v["end_ts"] = json!(end);
            v["active_secs"] = json!(secs);
            v["estimated"] = json!(estimated);
            v
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
    /// How long it took, when that is actually known. Omit it for reading that
    /// was never timed — a book, an article on a phone — and the duration is
    /// derived from the characters at your own recent pace instead.
    pub minutes: Option<f64>,
    /// Exact character count; when absent, pages × chars_per_page is used.
    pub chars: Option<i64>,
    pub pages: Option<f64>,
    pub work: Option<String>,
    pub source: Option<String>,
    pub note: Option<String>,
    /// Where the text came from — an article's URL.
    pub url: Option<String>,
    /// The text actually read. When present it *is* the character count.
    pub content: Option<String>,
}

/// When a hand-entered session started.
///
/// Shared with the book log, which resolves the same three ways: an explicit
/// timestamp, a date being back-filled, or nothing at all.
pub fn resolve_start_ts(
    start_ts: Option<f64>,
    date: Option<&str>,
    minutes: Option<f64>,
    settings: &db::Settings,
) -> Result<f64, AppError> {
    // An untimed session has no span to walk back over, so it anchors at the
    // moment it was logged.
    let ends_now = now_ts() - minutes.unwrap_or(0.0) * 60.0;
    let tz = crate::clock::tz_offset_secs();
    let today = stats::date_key(now_ts(), settings.day_rollover_hour, tz);

    Ok(match (start_ts, date) {
        (Some(ts), _) => ts,
        (None, Some(d)) => {
            let date = d
                .parse::<NaiveDate>()
                .map_err(|_| AppError::BadRequest(format!("bad date: {d}")))?;
            // mid-day anchor: rollover hour + 8h (12:00 local at the default 04)
            let midday = stats::day_start_ts(date, settings.day_rollover_hour, tz) + 8.0 * 3600.0;
            // "Today" is not a date being back-filled, it is now — and the form
            // pre-fills it, so this is the common path. Anchoring it at mid-day
            // would put an evening's reading in the morning of the timeline.
            //
            // The `midday > now` arm is the same case seen from the other side:
            // between midnight and the 04:00 rollover the browser's calendar
            // date is already tomorrow while the reading day is still today, so
            // the pre-filled date is one ahead and its mid-day anchor lies in
            // the future. Reading cannot have happened later than now.
            if date == today || midday > now_ts() {
                ends_now
            } else {
                midday
            }
        }
        (None, None) => ends_now,
    })
}

pub async fn create_session(
    State(state): State<AppState>,
    Json(req): Json<CreateSession>,
) -> Result<Json<db::ManualSession>, AppError> {
    // Negated, not `m <= 0.0`: every NaN comparison is false, so the flipped
    // form would let a NaN through as a valid duration.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    if req.minutes.is_some_and(|m| !(m > 0.0)) {
        return Err(AppError::BadRequest("minutes must be > 0".into()));
    }
    let settings = db::load_settings(&state.local).await?;

    // Content wins: when the text is here the count is exact, and it is
    // counted by the same rule as a hooked line so the speeds compare.
    // Then an explicitly given count, then pages × the estimate.
    let content = req.content.as_deref().filter(|c| !c.trim().is_empty());
    let chars = match (content, req.chars, req.pages) {
        (Some(text), _, _) => jp_core::text::chars::count_chars(text),
        (None, Some(c), _) if c >= 0 => c,
        (None, None, Some(p)) if p > 0.0 => (p * settings.chars_per_page).round() as i64,
        _ => return Err(AppError::BadRequest("need content, chars or pages".into())),
    };

    let start_ts = resolve_start_ts(req.start_ts, req.date.as_deref(), req.minutes, &settings)?;

    let url = req.url.as_deref().filter(|u| !u.trim().is_empty());
    // An article says what it is: it arrived with a URL or with its text.
    let default_source = if url.is_some() || content.is_some() {
        "article"
    } else {
        "book"
    };

    let session = db::insert_session(
        &state.knowledge,
        db::NewSession {
            start_ts,
            end_ts: req.minutes.map(|m| start_ts + m * 60.0),
            chars,
            source: req.source.as_deref().unwrap_or(default_source),
            work: req.work.as_deref(),
            pages: req.pages,
            note: req.note.as_deref(),
            url,
            content,
        },
    )
    .await?;
    Ok(Json(session))
}

/// The text a logged session was counted from — fetched on demand, never with
/// the session list. See `db::sessions`.
pub async fn session_content(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    let content = db::fetch_content(&state.knowledge, id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(json!({ "id": id, "content": content })))
}

#[derive(Deserialize)]
pub struct CountText {
    pub content: String,
}

/// Count a block of text the way a session would count it.
///
/// It exists so the log form can show the count *before* submitting without
/// reimplementing `count_chars` in JavaScript — which punctuation counts is a
/// rule this codebase keeps in exactly one place.
pub async fn count_text(Json(req): Json<CountText>) -> Json<Value> {
    Json(json!({ "chars": jp_core::text::chars::count_chars(&req.content) }))
}

pub async fn delete_session(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    if !db::delete_session(&state.knowledge, id).await? {
        return Err(AppError::NotFound);
    }
    Ok(Json(json!({ "deleted": id })))
}
