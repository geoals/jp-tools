//! `/api/vocab/*` — the knowledge ledger's status endpoints.
//!
//! Reads, the rebuild, and the triage pass that fills `status`
//! (`spec/cold-start.md` Pass 2, over terms already in the ledger). The ledger
//! itself is `jp_core::knowledge::vocabulary`.
//!
//! The one rule these handlers exist to keep: **`status` is only ever written
//! from a request the reader made.** No sync touches it, so the ledger cannot
//! demote a word behind their back and an encounter count cannot promote one.

use axum::Json;
use axum::extract::{Query, State};
use jp_core::knowledge::vocabulary::{self, Status, Term};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::clock::now_ts;
use crate::db;
use crate::error::AppError;

/// Rows per queue page. A batch big enough to be worth one sweep of attention
/// and small enough that submitting it is not a big commitment.
const QUEUE_LIMIT: i64 = 200;

/// What the ledger currently holds, by status — the numbers the seed page and
/// the vocabulary-size figure are built on.
///
/// `in_master` is the vocabulary scale: a term counts toward "I know N words"
/// only if the master dictionary lists it, because Jitendex's 400k entries are
/// a phrase index and would make the number meaningless
/// (`spec/knowledge-db.md`).
pub async fn vocab_summary(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let counts = vocabulary::status_counts(&state.knowledge).await?;
    let by_status: Vec<Value> = counts
        .iter()
        .map(|c| json!({ "status": c.status, "total": c.total, "in_master": c.in_master }))
        .collect();
    let total: i64 = counts.iter().map(|c| c.total).sum();
    let known: i64 = counts
        .iter()
        .filter(|c| c.status == "known")
        .map(|c| c.in_master)
        .sum();

    Ok(Json(json!({
        "total": total,
        "known_in_master": known,
        "by_status": by_status,
    })))
}

#[derive(Deserialize)]
pub struct QueueParams {
    /// Overrides the `triage_min_encounters` setting for one request, so the UI
    /// can preview what a threshold change does before saving it.
    min_encounters: Option<i64>,
}

/// The triage queue: untriaged vocabulary to judge, most-encountered first.
///
/// `preselect` is computed here rather than in the client. It is the rule the
/// whole seeding pass rests on, it has to be testable without a browser, and a
/// client-side copy would mean the threshold actually applied was recorded
/// nowhere.
pub async fn vocab_queue(
    State(state): State<AppState>,
    Query(params): Query<QueueParams>,
) -> Result<Json<Value>, AppError> {
    let settings = db::load_settings(&state.local).await?;
    let min = params
        .min_encounters
        .unwrap_or(settings.triage_min_encounters)
        .max(1);

    let rows = vocabulary::triage_queue(&state.knowledge, min, QUEUE_LIMIT).await?;
    let (pending, pending_preselected) = vocabulary::triage_pending(&state.knowledge, min).await?;

    let terms: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "headword": r.term.headword,
                "reading": r.term.display_reading(),
                "pos": r.pos,
                "encounter_count": r.encounter_count,
                "lookup_count": r.lookup_count,
                "mined": r.mined,
                "preselect": vocabulary::preselects_known(r, min),
            })
        })
        .collect();

    Ok(Json(json!({
        "min_encounters": min,
        "pending": pending,
        "pending_preselected": pending_preselected,
        "terms": terms,
    })))
}

#[derive(Deserialize)]
pub struct Judgement {
    headword: String,
    #[serde(default)]
    reading: String,
    status: String,
}

#[derive(Deserialize)]
pub struct JudgeRequest {
    judgements: Vec<Judgement>,
}

/// Write a batch of judgements — the triage submit.
///
/// Statuses are parsed strictly rather than through `Status::parse`, which
/// falls back to `new`. Here that fallback would be a silent data loss: a typo
/// in one row would quietly un-judge it while the response claimed the batch
/// landed.
pub async fn vocab_judge(
    State(state): State<AppState>,
    Json(req): Json<JudgeRequest>,
) -> Result<Json<Value>, AppError> {
    let mut judgements = Vec::with_capacity(req.judgements.len());
    for j in &req.judgements {
        let status = Status::ALL
            .iter()
            .copied()
            .find(|s| s.as_str() == j.status)
            .ok_or_else(|| AppError::BadRequest(format!("unknown status: {}", j.status)))?;
        if j.headword.is_empty() {
            return Err(AppError::BadRequest("empty headword".into()));
        }
        judgements.push((Term::new(j.headword.clone(), &j.reading), status));
    }

    let written = vocabulary::set_status_each(&state.knowledge, &judgements, now_ts()).await?;
    Ok(Json(json!({ "written": written })))
}

/// Blacklist every untriaged row no dictionary recognizes as a word.
///
/// The queue filters these out; this is what clears them, so the ledger's
/// untriaged count means "vocabulary still to judge" rather than being padded
/// by tokenizer noise.
pub async fn vocab_blacklist_non_words(
    State(state): State<AppState>,
) -> Result<Json<Value>, AppError> {
    let n = vocabulary::blacklist_non_words(&state.knowledge, now_ts()).await?;
    Ok(Json(json!({ "blacklisted": n })))
}

/// Rebuild the ledger's counts from the whole reading history.
///
/// Zeroes the aggregates, rewinds only the ledger's watermarks, and re-runs
/// both ingests — `word_days` is untouched, because its own watermarks stay
/// where they were. Assertions survive: `status` is not a count.
///
/// This exists because the ledger arrived years into a line history that was
/// already being tokenized for something else. It stays afterwards as the
/// repair path for a re-tokenization (a Sudachi upgrade, a change to what
/// counts as a content word), which is a thing that will happen again.
pub async fn vocab_rebuild(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    crate::ingest::reset_vocabulary(&state).await?;
    let lines = crate::ingest::ingest_new_lines(&state).await?;
    let sessions = crate::ingest::ingest_new_sessions(&state).await?;
    let mined = crate::ingest::sync_vocabulary(&state).await?;
    // Anything the re-ingest did not touch is no longer in the reading — a
    // proper noun now that names are excluded, or a term the tokenizer splits
    // differently than it used to. Judged rows and mined rows are spared.
    let pruned = vocabulary::prune_untouched(&state.knowledge).await?;

    Ok(Json(json!({
        "lines": lines,
        "sessions": sessions,
        "mined_terms": mined,
        "pruned": pruned,
    })))
}
