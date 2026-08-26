//! The overlay popup's two endpoints, and the lookup a popup *is*.
//!
//! The assembly is [`jp_core::define`], shared with yt-mine. What stays here is
//! the part that is not a dictionary question: on this surface opening the popup
//! is the reader admitting they did not know a word, so it records a lookup —
//! and [`retract`] takes that back when the popup turns out to have been opened
//! to reach a button.

use axum::Json;
use axum::extract::{Query, State};
use jp_core::define::{Definition, Expansion};
use serde::Deserialize;

use crate::app::AppState;
use crate::error::AppError;

#[derive(Deserialize)]
pub struct DefineQuery {
    /// The ledger's headword, not the surface — see [`jp_core::define`].
    pub term: String,
    /// Narrows the entries where a spelling has several readings: 空 is そら or
    /// から and they are different words. Absent, every reading is returned.
    pub reading: Option<String>,
}

/// `GET /api/reader/define?term=<headword>&reading=<reading>`
pub async fn define(
    State(state): State<AppState>,
    Query(q): Query<DefineQuery>,
) -> Result<Json<Definition>, AppError> {
    let mut definition =
        jp_core::define::define(state.knowledge.pool(), &q.term, q.reading.as_deref()).await?;

    // The same row a Yomitan popup would have written. Recorded here rather
    // than in jp-core because on this surface the popup *is* the lookup —
    // there is no AnkiConnect duplicate-check passing through the proxy to
    // count, and no reading session in yt-mine for one to belong to.
    // `record` gates on a line having arrived recently and dedupes, so a
    // second click on the same word inside the window is one lookup.
    definition.lookup_id = crate::routes::ankiproxy::record(&state, &q.term).await;

    Ok(Json(definition))
}

#[derive(Deserialize)]
pub struct RetractRequest {
    /// From the [`Definition`] this popup was drawn from. Paired with the term
    /// so a stale id cannot delete an unrelated row.
    pub lookup_id: i64,
    pub term: String,
}

/// `POST /api/reader/lookup/retract` — this popup was not a lookup after all.
///
/// The mobile overlay has to put the known/unknown buttons *in* the popup,
/// because a finger has no side mouse buttons to carry them. Marking a word
/// known there means the popup was opened to reach the button, not to read the
/// definition, and the row recorded on open would otherwise say the reader did
/// not know a word they just asserted they know — the one figure the lookup
/// tax is measured from.
///
/// Only `known` retracts. Marking a word *unknown* after reading its
/// definition is a lookup and stays one, and so does mining.
///
/// A lookup is also presence evidence, so the row is replaced by a reader mark
/// at the same instant: the popup was not a lookup, but the reader was
/// certainly at the screen when it opened.
pub async fn retract(
    State(state): State<AppState>,
    Json(req): Json<RetractRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ts = crate::db::retract_lookup(&state.knowledge, req.lookup_id, &req.term).await?;
    if let Some(ts) = ts
        && let Err(e) = crate::db::insert_reader_mark(&state.local, ts, "judge").await
    {
        tracing::warn!(error = %e, "retracted a lookup without leaving its presence mark");
    }
    Ok(Json(serde_json::json!({ "retracted": ts.is_some() })))
}

#[derive(Deserialize)]
pub struct ExpandQuery {
    /// The line from the clicked word's first character to its end.
    pub text: String,
}

/// `GET /api/reader/expand?text=<rest of the line>` — every other reading of
/// this position. See [`jp_core::define::expand`].
pub async fn expand(
    State(state): State<AppState>,
    Query(q): Query<ExpandQuery>,
) -> Result<Json<Vec<Expansion>>, AppError> {
    // The reader's own tokenizer, already warm for the line feed — a second
    // pipeline answers differently, and both halves of the scan depend on the
    // answer matching the ledger.
    let highlighter = super::highlight::shared(&state).await;
    Ok(Json(
        jp_core::define::expand(&state.knowledge, highlighter, &q.text).await?,
    ))
}
