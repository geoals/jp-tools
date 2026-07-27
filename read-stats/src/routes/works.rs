//! `/api/works` — per-VN totals, and the metadata attached to a title.
//!
//! Works join by exact title (see [`crate::db::works`]), so this endpoint is a
//! left join done in memory: aggregate the line stream by title, then attach
//! whatever metadata row shares that title. Titles with metadata but no lines
//! yet (a queued VN) still get a row, or the queue would be invisible until
//! reading starts.

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::db;
use crate::error::AppError;
use crate::history::History;
use crate::stats;
use jp_core::knowledge::work_terms;

/// How many words each per-work list carries. Short enough to read in one
/// pass — a list of two hundred unknown words is a wall, not a plan.
const UNKNOWN_LEN: i64 = 40;
const DISTINCTIVE_LEN: i64 = 24;

fn term_json(t: &work_terms::WorkTerm) -> Value {
    json!({
        "headword": t.headword,
        "reading": t.reading,
        "pos": t.pos,
        "status": t.status,
        "count": t.count,
        "elsewhere": t.elsewhere,
    })
}

fn meta_json(m: &db::Work) -> Value {
    json!({
        "id": m.id,
        "total_chars": m.total_chars,
        "cover": m.cover_path.as_ref().map(|p| format!("/covers/{p}")),
        "status": m.status,
        "queue_pos": m.queue_pos,
        "vn_window": m.vn_window,
    })
}

pub async fn works(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let h = History::load(&state).await?;
    let mut agg =
        stats::aggregate_works(&h.work_lines(), &h.presence(), h.settings.session_gap_secs);

    // Manual sessions merge in by title.
    for s in &h.manual {
        // Articles collapse into one row — see `stats::work::ARTICLES_WORK`.
        let key = stats::work_key(&s.source, s.work.as_deref());
        let entry = agg.entry(key).or_insert_with(|| stats::WorkAgg {
            first_ts: s.start_ts,
            ..Default::default()
        });
        let secs = h.duration_of(s).0;
        entry.chars += s.chars;
        entry.active_secs += secs;
        entry.first_ts = entry.first_ts.min(s.start_ts);
        entry.last_ts = entry.last_ts.max(s.start_ts + secs);
    }

    // Metadata joins by exact title; leftovers (queued works with no lines
    // yet) still get a row so they show up before reading starts.
    let mut meta_by_title: BTreeMap<String, db::Work> = db::fetch_works_meta(&state.knowledge)
        .await?
        .into_iter()
        .map(|w| (w.title.clone(), w))
        .collect();

    let mut list: Vec<_> = agg
        .into_iter()
        .map(|(work, a)| {
            let meta = work.as_ref().and_then(|t| meta_by_title.remove(t));
            json!({
                "work": work,
                "chars": a.chars,
                "active_secs": a.active_secs,
                "first_read": h.date_of(a.first_ts).to_string(),
                "last_read": h.date_of(a.last_ts).to_string(),
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

/// `GET /api/works/detail?work=<title>` — one work's own reading history.
///
/// Keyed by title, not by id: title is the join key lines are stamped with,
/// and a work can have plenty of reading behind it and no `works` row at all
/// (nothing upserts one, and the synthetic `Articles` work can never have
/// one). Metadata is therefore optional here — the reading is what makes a
/// work real on this page.
///
/// The same derivations the dashboard runs over the whole stream, run over the
/// slice of it stamped with this title: the days it was read on, the sittings
/// those days were made of, and what each sitting cost. Nothing here is stored
/// — a threshold change re-reads the whole history under the new rule, as
/// everywhere else.
///
/// Manual sessions for the work merge in, but only ones logged with real
/// minutes contribute to *speed*: an untimed session's duration is derived
/// from the reader's pace, so it would report that pace back (see
/// [`History::duration_of`]).
pub async fn work_detail(
    State(state): State<AppState>,
    Query(params): Query<WorkDetailParams>,
) -> Result<Json<Value>, AppError> {
    let title = params.work.trim();
    if title.is_empty() {
        return Err(AppError::BadRequest("work required".into()));
    }
    let meta = db::fetch_works_meta(&state.knowledge)
        .await?
        .into_iter()
        .find(|w| w.title == title);
    let h = History::load(&state).await?;
    let lines = h.lines_of_work(title);
    let sessions = stats::derive_sessions(&lines, &h.presence(), h.settings.session_gap_secs);

    // Manual sessions logged against this title. Articles collapse under one
    // synthetic work, so the key has to go through `work_key` rather than the
    // raw column.
    let manual: Vec<&db::ManualSession> = h
        .manual
        .iter()
        .filter(|s| stats::work_key(&s.source, s.work.as_deref()).as_deref() == Some(title))
        .collect();

    // Days: line-derived and logged reading summed into one series. The detail
    // page charts the work's own reading days, not a calendar window, so a
    // work read in four sittings has four bars and no empty month around them.
    let mut days: BTreeMap<chrono::NaiveDate, (i64, f64)> = BTreeMap::new();
    for s in &sessions {
        let d = days.entry(h.date_of(s.start_ts)).or_default();
        d.0 += s.chars;
        d.1 += s.active_secs;
    }
    for s in &manual {
        let d = days.entry(h.date_of(s.start_ts)).or_default();
        d.0 += s.chars;
        d.1 += h.duration_of(s).0;
    }

    let chars: i64 = days.values().map(|d| d.0).sum();
    let active_secs: f64 = days.values().map(|d| d.1).sum();
    // Speed divides by measured reading only.
    let measured_chars: i64 = sessions.iter().map(|s| s.chars).sum::<i64>()
        + manual
            .iter()
            .filter(|s| s.end_ts.is_some())
            .map(|s| s.chars)
            .sum::<i64>();
    let measured_secs: f64 = sessions.iter().map(|s| s.active_secs).sum::<f64>()
        + manual
            .iter()
            .filter(|s| s.end_ts.is_some())
            .map(|s| h.duration_of(s).0)
            .sum::<f64>();
    let speed = (measured_secs > 0.0).then(|| measured_chars as f64 / measured_secs * 3600.0);

    let sittings: Vec<Value> = sessions
        .iter()
        .map(|s| {
            json!({
                "start_ts": s.start_ts,
                "end_ts": s.end_ts,
                "date": h.date_of(s.start_ts).to_string(),
                "chars": s.chars,
                "active_secs": s.active_secs,
                "cards": h.cards_in(s.start_ts, s.end_ts),
                "estimated": false,
            })
        })
        .chain(manual.iter().map(|s| {
            let (secs, estimated) = h.duration_of(s);
            json!({
                "start_ts": s.start_ts,
                "end_ts": s.start_ts + secs,
                "date": h.date_of(s.start_ts).to_string(),
                "chars": s.chars,
                "active_secs": secs,
                "cards": 0,
                "estimated": estimated,
            })
        }))
        .collect();
    let mut sittings = sittings;
    // Newest first: the last time you sat down with it is the row you want.
    sittings.sort_by(|a, b| {
        b["start_ts"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&a["start_ts"].as_f64().unwrap_or(0.0))
    });

    // What the prose is like, and what the rest of your reading is like — one
    // pass, because the figure is only legible as a comparison. Pasted session
    // content counts: it is text that was read, and this asks what the writing
    // is, not what it cost.
    let mut mine = stats::ProseAcc::default();
    let mut rest = stats::ProseAcc::default();
    for (text, work) in db::fetch_line_texts(&state.knowledge).await? {
        if work.as_deref() == Some(title) {
            mine.push(&text);
        } else {
            rest.push(&text);
        }
    }
    for s in db::fetch_session_texts_after(&state.knowledge, 0).await? {
        let key = stats::work_key(&s.source, s.work.as_deref());
        if key.as_deref() == Some(title) {
            mine.push(&s.content);
        } else {
            rest.push(&s.content);
        }
    }

    // What the work is made of, and how much of it is already known. Empty
    // until the ingest has run over this work's text — `work_terms` is filled
    // by the same watermarked pass as the ledger.
    let vocab = work_terms::summary(&state.knowledge, title).await?;
    let unknown = work_terms::top_unknown(&state.knowledge, title, UNKNOWN_LEN).await?;
    let distinctive = work_terms::distinctive(&state.knowledge, title, DISTINCTIVE_LEN).await?;

    // What is left, at this work's own pace rather than the reader's average —
    // a work that reads slower than usual should say so in its own estimate.
    let remaining = meta
        .as_ref()
        .and_then(|m| m.total_chars)
        .map(|total| (total - chars).max(0));
    let remaining_secs = remaining
        .zip(speed)
        .filter(|(_, sp)| *sp > 0.0)
        .map(|(left, sp)| left as f64 / sp * 3600.0);

    if lines.is_empty() && manual.is_empty() && meta.is_none() {
        return Err(AppError::NotFound);
    }

    Ok(Json(json!({
        "work": title,
        "meta": meta.as_ref().map(meta_json),
        "chars": chars,
        "active_secs": active_secs,
        "speed": speed,
        "first_read": days.keys().next().map(|d| d.to_string()),
        "last_read": days.keys().next_back().map(|d| d.to_string()),
        "remaining_chars": remaining,
        "remaining_secs": remaining_secs,
        "days": days
            .iter()
            .map(|(date, (chars, secs))| json!({
                "date": date.to_string(),
                "chars": chars,
                "active_secs": secs,
            }))
            .collect::<Vec<_>>(),
        "sittings": sittings,
        "prose": mine.finish(),
        "corpus_prose": rest.finish(),
        "vocabulary": {
            "types": vocab.types,
            "tokens": vocab.tokens,
            "known_types": vocab.known_types,
            "known_tokens": vocab.known_tokens,
            "unknown_types": vocab.unknown_types,
            "new_types": vocab.new_types,
            "known_type_pct": vocab.known_type_pct(),
            "known_token_pct": vocab.known_token_pct(),
        },
        "top_unknown": unknown.iter().map(term_json).collect::<Vec<_>>(),
        "distinctive": distinctive.iter().map(term_json).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
pub struct WorkDetailParams {
    pub work: String,
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
    /// Substring of the VN's window title for screenshot capture. Empty
    /// string clears it (fall back to the focused window).
    pub vn_window: Option<String>,
}

/// Apply the optional fields of a metadata request to an existing work row,
/// doing the one-shot VNDB cover fetch when a vndb id is given.
async fn apply_work_meta(state: &AppState, id: i64, req: &WorkMetaReq) -> Result<(), AppError> {
    if let Some(raw) = &req.vndb_id {
        let old_cover = db::fetch_work(&state.knowledge, id)
            .await?
            .and_then(|w| w.cover_path);
        let new_cover = if raw.trim().is_empty() {
            db::set_work_cover(&state.knowledge, id, None).await?;
            db::clear_work_cover_vndb(&state.local, id).await?;
            None
        } else {
            let vid = crate::services::vndb::normalize_id(raw)
                .ok_or_else(|| AppError::BadRequest(format!("bad vndb id: {raw}")))?;
            // Fetches the cover and records both the filename (on the work) and
            // the vndb id (in work_covers), so a lost file can be re-fetched.
            Some(
                crate::services::covers::fetch_and_store(
                    &state.http,
                    &state.local,
                    &state.knowledge,
                    &state.covers_dir,
                    id,
                    &vid,
                )
                .await?,
            )
        };
        if let Some(old) = old_cover.filter(|old| Some(old) != new_cover.as_ref()) {
            let _ = tokio::fs::remove_file(state.covers_dir.join(&old)).await;
        }
    }
    if let Some(total) = req.total_chars {
        if total < 0 {
            return Err(AppError::BadRequest("total_chars must be >= 0".into()));
        }
        db::set_work_total_chars(&state.knowledge, id, (total > 0).then_some(total)).await?;
    }
    if let Some(status) = &req.status {
        if !db::WORK_STATUSES.contains(&status.as_str()) {
            return Err(AppError::BadRequest(format!(
                "status must be one of {:?}",
                db::WORK_STATUSES
            )));
        }
        db::set_work_status(&state.knowledge, id, status).await?;
    }
    if let Some(pos) = req.queue_pos {
        db::set_work_queue_pos(&state.knowledge, id, (pos >= 0).then_some(pos)).await?;
    }
    if let Some(win) = &req.vn_window {
        let win = win.trim();
        db::set_work_vn_window(&state.knowledge, id, (!win.is_empty()).then_some(win)).await?;
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
    let work = db::upsert_work(&state.knowledge, title).await?;
    apply_work_meta(&state, work.id, &req).await?;
    Ok(Json(
        db::fetch_work(&state.knowledge, work.id)
            .await?
            .ok_or(AppError::NotFound)?,
    ))
}

pub async fn update_work(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<WorkMetaReq>,
) -> Result<Json<db::Work>, AppError> {
    db::fetch_work(&state.knowledge, id)
        .await?
        .ok_or(AppError::NotFound)?;
    apply_work_meta(&state, id, &req).await?;
    Ok(Json(
        db::fetch_work(&state.knowledge, id)
            .await?
            .ok_or(AppError::NotFound)?,
    ))
}

pub async fn delete_work(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    let work = db::fetch_work(&state.knowledge, id)
        .await?
        .ok_or(AppError::NotFound)?;
    if let Some(cover) = &work.cover_path {
        let _ = tokio::fs::remove_file(state.covers_dir.join(cover)).await;
    }
    db::clear_work_cover_vndb(&state.local, id).await?;
    db::delete_work(&state.knowledge, id).await?;
    Ok(Json(json!({ "deleted": id })))
}
