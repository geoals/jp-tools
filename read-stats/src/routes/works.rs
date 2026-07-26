//! `/api/works` — per-VN totals, and the metadata attached to a title.
//!
//! Works join by exact title (see [`crate::db::works`]), so this endpoint is a
//! left join done in memory: aggregate the line stream by title, then attach
//! whatever metadata row shares that title. Titles with metadata but no lines
//! yet (a queued VN) still get a row, or the queue would be invisible until
//! reading starts.

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::db;
use crate::error::AppError;
use crate::history::History;
use crate::stats;

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
    let mut agg = stats::aggregate_works(
        &h.work_lines(),
        h.settings.afk_secs,
        h.settings.session_gap_secs,
    );

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
