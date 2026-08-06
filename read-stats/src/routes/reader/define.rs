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
//!
//! [`expand`] is the way out of that when the tokenizer split a word: it scans
//! the raw line to the right of the clicked position, so the reader can pick
//! the compound out of a segmentation nothing downstream could have joined.

use axum::Json;
use axum::extract::{Query, State};
use jp_core::knowledge::dictionaries;
use jp_core::knowledge::vocabulary::{self, Status, Term};
use serde::{Deserialize, Serialize};

use crate::app::AppState;
use crate::error::AppError;
use crate::ingest::READER_FREQUENCY;

/// The frequency list that ranks a *spelling with a reading*. Reader-facing
/// ranks come from [`READER_FREQUENCY`]; this one is shown beside it because
/// the two disagree loudly and the difference is informative — 船舶 is 3,843 in
/// newspaper prose against 32,370 in fiction.
const CORPUS_FREQUENCY: &str = "BCCWJ";

/// The dictionary the popup opens on, ahead of the master. Named rather than
/// derived: install order is the order zips were first seen, which says nothing
/// about which definition is worth reading first.
const OPENS_WITH: [&str; 1] = ["明鏡国語辞典 第三版"];

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
    /// The lookup row this popup was recorded as, for the client to hand back
    /// if the popup turns out not to have been a lookup — see [`retract`].
    /// `None` when nothing was recorded (outside a reading session).
    pub lookup_id: Option<i64>,
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
    // Meikyou, then the master, then everything else in install order — a
    // stable sort, so the last tier keeps it. The master decides what counts as
    // a word and is the vocabulary scale; that is not the same question as
    // which definition to read first, so the order is named here rather than
    // taken from the role.
    sources.sort_by_key(|s| {
        if OPENS_WITH.contains(&s.dictionary.as_str()) {
            0
        } else if dicts
            .iter()
            .any(|d| d.title == s.dictionary && d.role == dictionaries::Role::Master)
        {
            1
        } else {
            2
        }
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
    let lookup_id = crate::routes::ankiproxy::record(&state, &q.term).await;

    Ok(Json(Definition {
        term: q.term.clone(),
        jiten: rank_from(READER_FREQUENCY).await,
        bccwj: rank_from(CORPUS_FREQUENCY).await,
        sources,
        pitch,
        lookup_id,
    }))
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

/// One `(term, reading)` a dictionary offers for this position — a candidate
/// the popup can be re-opened on, not a match of the text: `term` is the
/// dictionary's spelling and the line may have conjugated it.
#[derive(Serialize)]
pub struct Expansion {
    pub term: String,
    /// The ledger key this spelling resolves to — what judging or mining the
    /// candidate writes, while `term` stays what the dictionary calls it. A
    /// dictionary headword is text from outside the tokenizer: 素振 is
    /// Jitendex's spelling and the ledger's row is 素振り, and writing the raw
    /// string would make a second row that reads as never encountered.
    pub key: String,
    /// One entry per reading, since 素振り is そぶり or すぶり and they are
    /// different words. Empty for a kana headword, which reads as itself.
    pub reading: String,
    /// What the ledger already says about `(key, reading)`. The popup lights
    /// its ✓ or ✗ from this, so re-opening a candidate shows the judgement
    /// made on it last time — without it a term judged a moment ago comes back
    /// looking untouched, which reads as the write having failed.
    pub status: String,
    pub dictionaries: Vec<String>,
}

/// How far to the right a scan reaches, in characters. Yomitan's own default.
const EXPAND_MAX_CHARS: usize = 10;

/// And in tokens, for the deinflected candidates. Five reaches the shape those
/// are for — noun + particle + verb, with room for a second particle.
const EXPAND_MAX_TOKENS: usize = 5;

/// `GET /api/reader/expand?text=<rest of the line>` — every other reading of
/// this position.
///
/// Two failures, one answer. The tokenizer splits 経年劣化 into 経年 and 劣化,
/// and both halves are right; the compound is a Jitendex headword and not a
/// Sankoku one, so no amount of dictionary validation would have joined them.
/// And where a spelling has several readings it picks one — 素振り as すぶり
/// where the line means そぶり — which is a different word with a different
/// ledger row. Both are the reader seeing what the pipeline cannot, so both get
/// the escape hatch Yomitan gave for free: take the text from the word's first
/// character, and list every `(term, reading)` any dictionary lists for a prefix
/// of it, longest first.
///
/// Candidates come in two shapes, because the two failures do. A literal
/// prefix of the line catches the split compound. A prefix ending on a token
/// boundary with its last token in canonical form
/// ([`super::highlight::Highlighter::prefix_forms`]) catches the conjugated
/// expression — しびれを切らした is listed as しびれを切らす, which no literal
/// prefix of the line spells.
pub async fn expand(
    State(state): State<AppState>,
    Query(q): Query<ExpandQuery>,
) -> Result<Json<Vec<Expansion>>, AppError> {
    let pool = state.knowledge.pool();
    // The reader's own tokenizer, already warm for the line feed — the second
    // pipeline a bare `SudachiTokenizer` would be answers differently, and
    // both halves of this depend on the answer matching the ledger.
    let highlighter = super::highlight::shared(&state).await;

    let chars: Vec<char> = q.text.chars().take(EXPAND_MAX_CHARS).collect();
    let mut candidates: Vec<String> = (2..=chars.len())
        .map(|n| chars[..n].iter().collect())
        .collect();
    // Two kinds of candidate. A literal prefix finds a compound the tokenizer
    // split; a deinflected one finds an expression the sentence conjugated —
    // しびれを切らした is しびれを切らす in every dictionary that has it, and no
    // prefix of the line spells that.
    if let Some(h) = &highlighter {
        let (h, text) = (h.clone(), q.text.clone());
        let forms = tokio::task::spawn_blocking(move || h.prefix_forms(&text, EXPAND_MAX_TOKENS))
            .await
            .unwrap_or_default();
        candidates.extend(forms.into_iter().filter(|f| f.chars().count() > 1));
    }
    candidates.sort();
    candidates.dedup();
    if candidates.is_empty() {
        return Ok(Json(Vec::new()));
    }

    // One statement per dictionary, keyed on `dictionary_id` so it seeks
    // `idx_dictionary_entries_lookup` — there is no index on `term` alone, and
    // a scan of 400k rows here would hold the write lock off every other
    // writer.
    let placeholders = vec!["?"; candidates.len()].join(",");
    let sql = format!(
        "SELECT DISTINCT term, reading FROM dictionary_entries INDEXED BY idx_dictionary_entries_lookup \
         WHERE dictionary_id = ? AND term IN ({placeholders})"
    );

    let mut found: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
    for dict in dictionaries::list_dictionaries(pool).await? {
        let mut query = sqlx::query_as::<_, (String, String)>(&sql).bind(dict.id);
        for c in &candidates {
            query = query.bind(c);
        }
        for (term, reading) in query.fetch_all(pool).await? {
            let slot = match found.iter_mut().find(|(t, _, _)| t == &term) {
                Some(slot) => slot,
                None => {
                    found.push((term, Vec::new(), Vec::new()));
                    found.last_mut().expect("just pushed")
                }
            };
            if !reading.is_empty() && !slot.1.contains(&reading) {
                slot.1.push(reading);
            }
            if !slot.2.contains(&dict.title) {
                slot.2.push(dict.title.clone());
            }
        }
    }

    found.sort_by_key(|(term, _, _)| std::cmp::Reverse(term.chars().count()));

    // A key resolved by a second pipeline matches no ledger row. With no
    // tokenizer at all, every candidate keys on itself, which is what the
    // ledger did before this existed.
    let keys = match highlighter {
        Some(h) => {
            let terms: Vec<String> = found.iter().map(|(t, _, _)| t.clone()).collect();
            tokio::task::spawn_blocking(move || {
                terms.iter().map(|t| h.ledger_key(t)).collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default()
        }
        None => Vec::new(),
    };

    // One candidate per reading, keyed as the ledger keys it.
    let candidates: Vec<(String, String, String, Vec<String>)> = found
        .into_iter()
        .enumerate()
        .flat_map(|(i, (term, readings, dictionaries))| {
            let key = keys.get(i).cloned().unwrap_or_else(|| term.clone());
            let readings = if readings.is_empty() {
                vec![String::new()]
            } else {
                readings
            };
            readings
                .into_iter()
                .map(|reading| {
                    (term.clone(), key.clone(), reading, dictionaries.clone())
                })
                .collect::<Vec<_>>()
        })
        .collect();

    let terms: Vec<Term> = candidates
        .iter()
        .map(|(_, key, reading, _)| Term::new(key.clone(), reading))
        .collect();
    let rows = vocabulary::fetch_many(&state.knowledge, &terms).await?;

    Ok(Json(
        candidates
            .into_iter()
            .zip(terms)
            .map(|((term, key, reading, dictionaries), t)| Expansion {
                status: rows
                    .get(&t)
                    .map_or(Status::New, |r| r.status)
                    .as_str()
                    .to_string(),
                term,
                key,
                reading,
                dictionaries,
            })
            .collect(),
    ))
}
