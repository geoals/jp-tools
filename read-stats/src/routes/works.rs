//! `/api/works` — per-VN totals, and the metadata attached to a title.
//!
//! Works join by exact title (see [`crate::db::works`]), so this endpoint is a
//! left join done in memory: aggregate the line stream by title, then attach
//! whatever metadata row shares that title. Titles with metadata but no lines
//! yet (a planned VN) still get a row, or the queue would be invisible until
//! reading starts.

use std::collections::{BTreeMap, HashSet};

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::db;
use crate::error::AppError;
use crate::history::History;
use crate::stats;
use jp_core::knowledge::{work_scripts, work_terms};

/// How many words each per-work list carries. Short enough to read in one
/// pass — a list of two hundred unknown words is a wall, not a plan.
const MINED_LEN: usize = 24;

/// What a work is, for the library filter. Derivable rather than stored: an
/// epub settles it outright, the texthooker only ever stamps VNs, and
/// everything else entered the library by hand, where the log form says what
/// it was. The synthetic Articles work is its own kind, because it is a
/// bucket, not a thing being read through.
///
/// The epub has to be asked first, and before the sources are consulted at
/// all: a book whose epub is uploaded but which has had no sitting logged yet
/// has no sources to go on, and an empty list satisfies every `all` test.
/// A work with nothing behind it at all is taken for a VN — that is what the
/// queue is made of, and one sitting corrects it.
fn work_kind(
    title: &str,
    has_lines: bool,
    has_epub: bool,
    manual_sources: &[String],
) -> &'static str {
    if title == stats::ARTICLES_WORK {
        "articles"
    } else if has_epub {
        "book"
    } else if has_lines
        || manual_sources
            .iter()
            .all(|s| matches!(s.as_str(), "vn" | "article"))
    {
        "vn"
    } else {
        "book"
    }
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

    // What each title is, for the library's kind filter: the set of titles
    // the texthooker stamped (so they are VNs) and the sources of the manual
    // sessions each title aggregated (books were logged by hand).
    let line_titles: HashSet<String> = h
        .work_lines()
        .iter()
        .filter_map(|l| l.work.clone())
        .collect();
    let mut manual_sources: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for s in &h.manual {
        if let Some(key) = stats::work_key(&s.source, s.work.as_deref()) {
            manual_sources
                .entry(key)
                .or_default()
                .push(s.source.clone());
        }
    }
    let epub_titles: HashSet<String> = db::fetch_books(&state.knowledge)
        .await?
        .into_iter()
        .map(|b| b.work)
        .collect();
    let kind_of = |title: &str| {
        work_kind(
            title,
            line_titles.contains(title),
            epub_titles.contains(title),
            manual_sources.get(title).map(Vec::as_slice).unwrap_or(&[]),
        )
    };

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
                "kind": work.as_deref().map(&kind_of),
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
            "kind": kind_of(&title),
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
    // The same days over measured reading only, which is what a per-day speed
    // may divide by: an untimed session's duration came from the pace.
    let mut measured_days: BTreeMap<chrono::NaiveDate, (i64, f64)> = BTreeMap::new();
    for s in &sessions {
        let d = days.entry(h.date_of(s.start_ts)).or_default();
        d.0 += s.chars;
        d.1 += s.active_secs;
        let m = measured_days.entry(h.date_of(s.start_ts)).or_default();
        m.0 += s.chars;
        m.1 += s.active_secs;
    }
    for s in &manual {
        let d = days.entry(h.date_of(s.start_ts)).or_default();
        d.0 += s.chars;
        d.1 += h.duration_of(s).0;
        if s.end_ts.is_some() {
            let m = measured_days.entry(h.date_of(s.start_ts)).or_default();
            m.0 += s.chars;
            m.1 += h.duration_of(s).0;
        }
    }

    // Speed with the lookups taken out: characters whose own gap held no
    // lookup, over the time those gaps cost. Both sides drop together — see
    // [`stats::Bucket::clean_chars`]. The bucket width is arbitrary here since
    // this only ever re-aggregates to whole days.
    let mut clean_days: BTreeMap<chrono::NaiveDate, (i64, f64)> = BTreeMap::new();
    for b in stats::bucket_lines(
        &lines,
        &h.lookups,
        &h.presence(),
        h.settings.session_gap_secs,
        60.0,
    ) {
        let d = clean_days.entry(h.date_of(b.t)).or_default();
        d.0 += b.clean_chars;
        d.1 += b.active_secs - b.lookup_secs;
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

    // The work's vocabulary twice over: what has been met in it so far, and
    // what its whole script holds. The pair is the figure — met-so-far alone
    // is a sample of the work drawn by how far you happen to have got.
    //
    // `work_terms` fills line by line behind the ingest watermark, so it is
    // empty until the ingest has run. `work_scripts` is empty until the script
    // has been imported (`jp-script profile`), which most works never will be.
    let vocab = work_terms::summary(&state.knowledge, title).await?;
    let script = work_scripts::coverage(&state.knowledge, title).await?;
    let met_types = work_scripts::met_types(&state.knowledge, title).await?;

    // What the work gave back. Note ids are epoch milliseconds, so a card
    // attributes to whatever was on screen when it was added — the same
    // nearest-line test the lookup guard uses, applied to ask *whose* reading
    // rather than whether it was reading at all.
    let mut mined: Vec<Value> = Vec::new();
    let mut cards_per_day: BTreeMap<chrono::NaiveDate, i64> = BTreeMap::new();
    for note in db::fetch_anki_notes(&state.knowledge).await? {
        let ts = note.note_id as f64 / 1000.0;
        if h.work_at(ts) != Some(title) {
            continue;
        }
        *cards_per_day.entry(h.date_of(ts)).or_default() += 1;
        mined.push(json!({ "vocab": note.vocab, "ts": ts }));
    }
    // Newest first, and only the tail is listed: the count is the figure, the
    // list is a reminder of what it was made of.
    mined.reverse();
    let mined_count = mined.len();
    mined.truncate(MINED_LEN);

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
                "cards": cards_per_day.get(date).copied().unwrap_or(0),
                "clean": clean_days.get(date).map(|(c, secs)| json!({
                    "chars": c,
                    "active_secs": secs,
                })),
                "measured": measured_days.get(date).map(|(c, secs)| json!({
                    "chars": c,
                    "active_secs": secs,
                })),
            }))
            .collect::<Vec<_>>(),
        "sittings": sittings,
        "vocabulary": {
            "types": vocab.types,
            "tokens": vocab.tokens,
            "known_types": vocab.known_types,
            "known_tokens": vocab.known_tokens,
            "known_type_pct": vocab.known_type_pct(),
            "known_token_pct": vocab.known_token_pct(),
        },
        // Absent rather than zeroed when no script has been imported: the page
        // has to tell "none of it is known" from "nothing has been counted".
        "script": (script.types > 0).then(|| json!({
            "types": script.types,
            "tokens": script.tokens,
            "known_types": script.known_types,
            "known_tokens": script.known_tokens,
            "known_type_pct": script.known_types as f64 / script.types as f64 * 100.0,
            "known_token_pct": script.token_coverage() * 100.0,
            "met_types": met_types,
        })),
        "mined_count": mined_count,
        "mined": mined,
    })))
}

#[derive(Deserialize)]
pub struct WorkDetailParams {
    pub work: String,
}

/// How many terms a session loads at once. Long enough to sit down to, short
/// enough that the queue is re-derived often — one judgement can take another
/// term off it (a headword judged under one reading takes every reading of it),
/// and the next fetch is where that is noticed.
const TRIAGE_LEN: i64 = 300;

/// `GET /api/works/triage?work=<title>` — the script's unjudged words,
/// commonest in this work first.
///
/// Distinct from the encounter-driven triage queue, and it has to be: that one
/// offers words the reader has met, ranked by how often. Most of a script is
/// words never met, which can never reach it.
///
/// Nothing here is preselected and nothing is written by loading it. A word
/// judged on sight has no encounters and no lookup record, which is exactly
/// what `preselects_known` requires before it will default anything to `known`.
pub async fn work_triage(
    State(state): State<AppState>,
    Query(params): Query<WorkDetailParams>,
) -> Result<Json<Value>, AppError> {
    let title = params.work.trim();
    if title.is_empty() {
        return Err(AppError::BadRequest("work required".into()));
    }
    let frequency = crate::routes::vocab::reader_frequency(&state).await?;
    let terms = work_scripts::triage_queue(&state.knowledge, title, frequency, TRIAGE_LEN).await?;
    Ok(Json(json!({
        "work": title,
        "terms": terms.iter().map(|t| json!({
            "headword": t.headword,
            "reading": t.reading,
            "count": t.count,
            "rank": t.rank,
            "met": t.met,
        })).collect::<Vec<_>>(),
    })))
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
