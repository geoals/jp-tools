//! `/api/vocab/*` — the knowledge ledger's status endpoints.
//!
//! Read-only plus the rebuild, for now. The triage UI (`spec/cold-start.md`'s
//! passes 1–3) and the seed importer write through here later; the ledger
//! itself is `jp_core::knowledge::vocabulary`.

use axum::Json;
use axum::extract::State;
use jp_core::knowledge::vocabulary;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::AppError;

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

    Ok(Json(json!({
        "lines": lines,
        "sessions": sessions,
        "mined_terms": mined,
    })))
}
