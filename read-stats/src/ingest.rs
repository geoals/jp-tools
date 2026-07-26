//! Incremental tokenization of everything read, into the two things a token is
//! worth keeping for.
//!
//! Runs on Anki refresh. One pass over the text produces both:
//!
//! - **`word_days`** — per-day content-word counts, which the kanji grid, the
//!   discovery curve and every coverage figure are derived from.
//! - **the `vocabulary` ledger** — `(headword, reading)` rows carrying running
//!   encounter counts, which the `#read` highlighter and i+1 marking look
//!   status up in. See `spec/knowledge-db.md`.
//!
//! Tokenization uses the mined vocab as Sudachi validation headwords: a mined
//! compound found whole in Mode C is kept whole (so it matches its card),
//! anything unrecognized is split down to finer modes.
//!
//! ## Two watermarks per stream, not one
//!
//! Each sink tracks its own last-processed id. That is what makes the ledger
//! backfillable: the ledger arrived years into a line history, so it has to be
//! filled from text `word_days` already counted, and a single shared watermark
//! would force a choice between an empty ledger and double-counted days. With
//! the watermarks separate, resetting only the ledger's re-tokenizes everything
//! and writes only the rows that are behind.
//!
//! Both sinks are additive and neither is idempotent, so the rule is absolute:
//! **a row is written to a sink only when its id is past that sink's
//! watermark.**

use std::collections::{HashMap, HashSet};

use jp_core::knowledge::vocabulary::{Encounter, Term};
use jp_core::tokenize::{SudachiTokenizer, Token, Tokenizer, is_content_word};
use tracing::{info, warn};

use crate::app::AppState;
use crate::db;
use crate::error::AppError;
use crate::stats;

const WATERMARK_KEY: &str = "tokenized_through_line_id";
/// Sessions get a watermark of their own rather than sharing the line one:
/// they are a separate id space, and a session logged today can carry text
/// read long before the newest line.
const SESSION_WATERMARK_KEY: &str = "tokenized_through_session_id";
/// The ledger's own pair, for the reason in the module doc.
const VOCAB_LINE_WATERMARK_KEY: &str = "vocab_through_line_id";
const VOCAB_SESSION_WATERMARK_KEY: &str = "vocab_through_session_id";

fn tz_offset_secs() -> i64 {
    chrono::Local::now().offset().local_minus_utc() as i64
}

#[derive(Debug, serde::Serialize)]
pub struct IngestOutcome {
    pub lines: usize,
    /// Distinct `(lemma, day)` pairs written to `word_days`.
    pub words: usize,
    /// Distinct terms whose ledger row was touched.
    pub terms: usize,
}

impl IngestOutcome {
    fn none() -> Self {
        IngestOutcome {
            lines: 0,
            words: 0,
            terms: 0,
        }
    }
}

/// Both sinks, accumulated over one tokenization pass.
///
/// Kept together so the tokenizer runs once: loading the Sudachi dictionary
/// costs more than tokenizing a day's lines, and the ledger backfill is a pass
/// over the whole history.
#[derive(Default)]
struct Harvest {
    /// `(lemma, day) → count`
    days: HashMap<(String, String), i64>,
    /// `term → (pos, count, first_ts, last_ts)`
    terms: HashMap<Term, (Option<String>, i64, f64, f64)>,
}

impl Harvest {
    /// Fold one token in, into whichever sinks this piece of text is behind on.
    fn add(&mut self, t: Token, date: &str, ts: f64, to_days: bool, to_terms: bool) {
        if !is_content_word(&t.pos) {
            return;
        }
        if to_days {
            *self
                .days
                .entry((t.base_form.clone(), date.to_string()))
                .or_default() += 1;
        }
        if to_terms {
            let entry = self
                .terms
                .entry(Term::new(t.base_form, &t.reading))
                .or_insert((Some(t.pos), 0, ts, ts));
            entry.1 += 1;
            entry.2 = entry.2.min(ts);
            entry.3 = entry.3.max(ts);
        }
    }

    fn day_rows(&self) -> Vec<(String, String, i64)> {
        self.days
            .iter()
            .map(|((lemma, date), count)| (lemma.clone(), date.clone(), *count))
            .collect()
    }

    fn encounters(&self) -> Vec<Encounter> {
        self.terms
            .iter()
            .map(|(term, (pos, count, first_ts, last_ts))| Encounter {
                term: term.clone(),
                pos: pos.clone(),
                count: *count,
                first_ts: *first_ts,
                last_ts: *last_ts,
            })
            .collect()
    }
}

/// Read a watermark, defaulting to 0 (nothing processed).
async fn watermark(state: &AppState, key: &str) -> Result<i64, AppError> {
    Ok(db::get_setting_raw(&state.local, key)
        .await?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0))
}

/// The mined deck, as Sudachi validation headwords.
async fn validation_headwords(state: &AppState) -> Result<HashSet<String>, AppError> {
    Ok(db::fetch_anki_notes(&state.knowledge)
        .await?
        .into_iter()
        .map(|n| n.vocab)
        .collect())
}

/// Write both sinks and advance both watermarks. `max_id` is the highest id in
/// the batch; each watermark only moves for the sink that was actually behind.
async fn commit(
    state: &AppState,
    harvest: &Harvest,
    max_id: i64,
    day_key: &str,
    vocab_key: &str,
) -> Result<(usize, usize), AppError> {
    let day_rows = harvest.day_rows();
    let encounters = harvest.encounters();
    db::add_word_day_counts(&state.knowledge, &day_rows).await?;
    jp_core::knowledge::vocabulary::record_encounters(&state.knowledge, &encounters).await?;
    db::save_setting(&state.local, day_key, &max_id.to_string()).await?;
    db::save_setting(&state.local, vocab_key, &max_id.to_string()).await?;
    Ok((day_rows.len(), encounters.len()))
}

pub async fn ingest_new_lines(state: &AppState) -> Result<IngestOutcome, AppError> {
    let day_mark = watermark(state, WATERMARK_KEY).await?;
    let vocab_mark = watermark(state, VOCAB_LINE_WATERMARK_KEY).await?;

    // Re-read from whichever sink is furthest behind; each token is then
    // credited only to the sinks that have not had it.
    let lines = db::fetch_lines_after(&state.knowledge, day_mark.min(vocab_mark)).await?;
    let Some(max_id) = lines.last().map(|l| l.id) else {
        return Ok(IngestOutcome::none());
    };

    let settings = db::load_settings(&state.local).await?;
    let rollover = settings.day_rollover_hour;
    let tz = tz_offset_secs();
    let vocab = validation_headwords(state).await?;
    let dict_path = state.sudachi_dict_path.clone();

    let n_lines = lines.len();
    // Dictionary load + tokenization are CPU-bound; keep them off the runtime.
    let harvest = tokio::task::spawn_blocking(move || -> Result<_, AppError> {
        let tokenizer = SudachiTokenizer::new(&dict_path, vocab)
            .map_err(|e| AppError::Upstream(format!("sudachi: {e}")))?;
        let mut harvest = Harvest::default();
        for line in &lines {
            let date = stats::date_key(line.ts, rollover, tz).to_string();
            let (to_days, to_terms) = (line.id > day_mark, line.id > vocab_mark);
            match tokenizer.tokenize(&line.text) {
                Ok(tokens) => {
                    for t in tokens {
                        harvest.add(t, &date, line.ts, to_days, to_terms);
                    }
                }
                Err(e) => warn!(line_id = line.id, error = %e, "tokenize failed, skipping line"),
            }
        }
        Ok(harvest)
    })
    .await
    .map_err(|e| AppError::Upstream(format!("tokenize task panicked: {e}")))??;

    let (words, terms) = commit(
        state,
        &harvest,
        max_id,
        WATERMARK_KEY,
        VOCAB_LINE_WATERMARK_KEY,
    )
    .await?;

    info!(lines = n_lines, words, terms, "line ingest complete");
    Ok(IngestOutcome {
        lines: n_lines,
        words,
        terms,
    })
}

/// The same pass over manually logged sessions that carry their text.
///
/// A session's words all land on the day its `start_ts` falls in — one date
/// for the whole row. There are no per-line timestamps to spread them over,
/// which is the same reason `content` lives on the session row instead of
/// being expanded into `lines`.
///
/// Reading an article genuinely re-shows you a word, so these counts belong in
/// `word_days` and in the ledger beside the hooked ones. This is deliberately
/// *not* true of the lookup rates, which stay over hooked reading only — see
/// `stats::rate` and `stats::kanji`.
pub async fn ingest_new_sessions(state: &AppState) -> Result<IngestOutcome, AppError> {
    let day_mark = watermark(state, SESSION_WATERMARK_KEY).await?;
    let vocab_mark = watermark(state, VOCAB_SESSION_WATERMARK_KEY).await?;

    let sessions =
        db::fetch_session_texts_after(&state.knowledge, day_mark.min(vocab_mark)).await?;
    let Some(max_id) = sessions.last().map(|s| s.id) else {
        return Ok(IngestOutcome::none());
    };

    let settings = db::load_settings(&state.local).await?;
    let rollover = settings.day_rollover_hour;
    let tz = tz_offset_secs();
    let vocab = validation_headwords(state).await?;
    let dict_path = state.sudachi_dict_path.clone();

    let n_sessions = sessions.len();
    let harvest = tokio::task::spawn_blocking(move || -> Result<_, AppError> {
        let tokenizer = SudachiTokenizer::new(&dict_path, vocab)
            .map_err(|e| AppError::Upstream(format!("sudachi: {e}")))?;
        let mut harvest = Harvest::default();
        for s in &sessions {
            let date = stats::date_key(s.start_ts, rollover, tz).to_string();
            let (to_days, to_terms) = (s.id > day_mark, s.id > vocab_mark);
            // Sentence by sentence: Sudachi has an input length limit, and an
            // article is far longer than the hooked line this path was built
            // for. Splitting keeps every token in the same analysis window it
            // would have had as a line.
            for sentence in jp_core::text::sentences::split_sentences(&s.content) {
                match tokenizer.tokenize(&sentence) {
                    Ok(tokens) => {
                        for t in tokens {
                            harvest.add(t, &date, s.start_ts, to_days, to_terms);
                        }
                    }
                    Err(e) => warn!(session_id = s.id, error = %e, "tokenize failed, skipping"),
                }
            }
        }
        Ok(harvest)
    })
    .await
    .map_err(|e| AppError::Upstream(format!("tokenize task panicked: {e}")))??;

    let (words, terms) = commit(
        state,
        &harvest,
        max_id,
        SESSION_WATERMARK_KEY,
        VOCAB_SESSION_WATERMARK_KEY,
    )
    .await?;

    info!(
        sessions = n_sessions,
        words, terms, "session ingest complete"
    );
    Ok(IngestOutcome {
        lines: n_sessions,
        words,
        terms,
    })
}

/// Rewind the ledger's watermarks so the next ingest re-reads the whole
/// history into `vocabulary`, without touching `word_days`.
///
/// This is the backfill: the ledger was added long after the line stream
/// started, and its counts are only true if the text that predates it is
/// tokenized too. Safe to run more than once — the ledger rows are recreated
/// from scratch, not added to, so a second run gives the same numbers as the
/// first. `status` survives it: the reset clears counts, and an assertion is
/// not a count.
pub async fn reset_vocabulary(state: &AppState) -> Result<(), AppError> {
    sqlx::query("UPDATE vocabulary SET encounter_count = 0, first_seen = NULL, last_seen = NULL")
        .execute(state.knowledge.pool())
        .await?;
    db::save_setting(&state.local, VOCAB_LINE_WATERMARK_KEY, "0").await?;
    db::save_setting(&state.local, VOCAB_SESSION_WATERMARK_KEY, "0").await?;
    info!("vocabulary ledger counts reset; next ingest will rebuild them");
    Ok(())
}

/// The wholesale syncs, run after an ingest: Anki owns `mined`, `lookups` owns
/// `lookup_count`, the dictionaries own the wordhood flags. None of the three
/// touches `status`.
pub async fn sync_vocabulary(state: &AppState) -> Result<i64, AppError> {
    let mined = jp_core::knowledge::vocabulary::sync_mined(&state.knowledge).await?;
    jp_core::knowledge::vocabulary::sync_lookup_counts(&state.knowledge).await?;
    jp_core::knowledge::vocabulary::refresh_dictionary_flags(&state.knowledge).await?;
    Ok(mined)
}
