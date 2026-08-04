//! What a word means, for the overlay's popup.
//!
//! The overlay draws over the VN in fullscreen, where Yomitan cannot reach —
//! `#read` is a browser page and has the popup for free, this one does not. So
//! the lookup Yomitan would have done is served here instead.
//!
//! Segmentation is not repeated: the SSE line event already carries a span per
//! word ([`super::highlight::Span`]), each with the `(headword, reading)` the
//! ledger keys on. The client sends that pair back, so the popup describes the
//! term the tokenizer decided on rather than the surface under the finger —
//! 振っ is looked up as 振る.

use axum::Json;
use axum::extract::{Query, State};
use jp_core::knowledge::dictionaries;
use serde::{Deserialize, Serialize};

use crate::app::AppState;
use crate::error::AppError;
use crate::ingest::READER_FREQUENCY;

/// The frequency list that ranks a *spelling with a reading*. Reader-facing
/// ranks come from [`READER_FREQUENCY`]; this one is shown beside it because
/// the two disagree loudly and the difference is informative — 船舶 is 3,843 in
/// newspaper prose against 32,370 in fiction.
const CORPUS_FREQUENCY: &str = "BCCWJ";

#[derive(Deserialize)]
pub struct DefineQuery {
    /// The ledger's headword, not the surface — see the module docs.
    pub term: String,
    /// Narrows the entries where a spelling has several readings: 空 is そら or
    /// から and they are different words. Absent, every reading is returned.
    pub reading: Option<String>,
}

#[derive(Serialize)]
pub struct Sense {
    pub reading: String,
    pub definitions: Vec<String>,
}

#[derive(Serialize)]
pub struct Source {
    pub dictionary: String,
    pub senses: Vec<Sense>,
}

#[derive(Serialize)]
pub struct Pitch {
    pub reading: String,
    /// Downstep positions: 0 is heiban, 1 atamadaka, n the mora it falls after.
    pub positions: Vec<u32>,
}

#[derive(Serialize)]
pub struct Definition {
    pub term: String,
    /// Ranked by [`READER_FREQUENCY`] — how common the word is in fiction.
    pub jiten: Option<i64>,
    /// Ranked by [`CORPUS_FREQUENCY`]. Taken over the spelling alone, so where
    /// a spelling has several readings this is the commonest of them.
    pub bccwj: Option<i64>,
    /// Master dictionary first, then the rest in install order. A dictionary
    /// holding nothing for this term is absent rather than empty.
    pub sources: Vec<Source>,
    /// From whichever installed dictionary carries pitch — NHK here. Narrowed
    /// to the asked-for reading when it lists it, since the accent is a
    /// property of the reading and 空/そら's is not 空/から's.
    pub pitch: Vec<Pitch>,
}

/// `GET /api/reader/define?term=<headword>&reading=<reading>`
pub async fn define(
    State(state): State<AppState>,
    Query(q): Query<DefineQuery>,
) -> Result<Json<Definition>, AppError> {
    let pool = state.knowledge.pool();
    let dicts = dictionaries::list_dictionaries(pool).await?;

    let rank_from = async |title: &str| match dicts.iter().find(|d| d.title == title) {
        Some(d) => dictionaries::lookup_frequency(pool, d.id, &q.term)
            .await
            .unwrap_or(None),
        None => None,
    };

    let mut sources = Vec::new();
    for dict in &dicts {
        let entries = dictionaries::lookup_dictionary_entries(pool, dict.id, &q.term).await?;
        // A frequency or pitch dictionary has no term entries at all, which is
        // what keeps BCCWJ, Jiten and NHK out of the popup without naming them.
        if entries.is_empty() {
            continue;
        }
        // Keep only the asked-for reading when the dictionary actually lists
        // it. When it doesn't, showing every reading beats showing nothing:
        // Sudachi and the dictionary can disagree about how a word is read.
        let matching: Vec<_> = match &q.reading {
            Some(r) if entries.iter().any(|e| &e.reading == r) => {
                entries.iter().filter(|e| &e.reading == r).collect()
            }
            _ => entries.iter().collect(),
        };
        sources.push(Source {
            dictionary: dict.title.clone(),
            senses: matching
                .into_iter()
                .map(|e| Sense {
                    reading: e.reading.clone(),
                    definitions: e.definitions.clone(),
                })
                .collect(),
        });
    }
    // The master is the definition the reader wants first; everything else is
    // there to fill the gaps it leaves.
    sources.sort_by_key(|s| {
        !dicts
            .iter()
            .any(|d| d.title == s.dictionary && d.role == dictionaries::Role::Master)
    });

    let mut pitch = Vec::new();
    for dict in &dicts {
        let entries = dictionaries::lookup_pitch_entries(pool, dict.id, &q.term).await?;
        if entries.is_empty() {
            continue;
        }
        pitch = entries
            .into_iter()
            .filter(|e| q.reading.as_ref().is_none_or(|r| &e.reading == r))
            .map(|e| Pitch {
                reading: e.reading,
                positions: e.positions,
            })
            .collect();
        break;
    }

    // The same row a Yomitan popup would have written. Recorded here
    // because on this surface the popup *is* the lookup — there is no
    // AnkiConnect duplicate-check passing through the proxy to count.
    // `record` gates on a line having arrived recently and dedupes, so a
    // second click on the same word inside the window is one lookup.
    crate::routes::ankiproxy::record(&state, &q.term).await;

    Ok(Json(Definition {
        term: q.term.clone(),
        jiten: rank_from(READER_FREQUENCY).await,
        bccwj: rank_from(CORPUS_FREQUENCY).await,
        sources,
        pitch,
    }))
}
