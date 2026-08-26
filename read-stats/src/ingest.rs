//! Incremental tokenization of everything read. Runs on Anki refresh; one pass
//! over the text fills three sinks:
//!
//! - **`word_days`** — per-day content-word counts, behind the kanji grid, the
//!   discovery curve and every coverage figure.
//! - **the `vocabulary` ledger** — `(headword, reading)` rows with running
//!   encounter counts, which `#read`'s highlighter looks status up in.
//! - **`work_terms`** — the same terms counted per work, so a work's page can
//!   say how much of it you already know.
//! - **`term_surfaces`** — how each term was actually spelt, with a line id per
//!   spelling, since the ledger's normalized key cannot say whether 窺う was
//!   read as 窺う or as うかがう.
//!
//! Tokenization uses the mined vocab beside the master lexicon, so a mined
//! compound found whole in Mode C stays whole and matches its card.
//!
//! **Each sink has its own watermark per stream, and a row is written to a sink
//! only when its id is past *that sink's* mark.** The sinks are additive and not
//! idempotent, so one shared watermark would force a choice between an empty
//! ledger and double-counted days. Separate, resetting one re-tokenizes
//! everything and writes only the rows that sink is behind on.

use std::collections::HashMap;

use jp_core::highlight::Wordhood;
use jp_core::knowledge::term_surfaces::SurfaceEncounter;
use jp_core::knowledge::vocabulary::{Encounter, Term};
use jp_core::knowledge::work_terms::WorkEncounter;
use jp_core::tokenize::{MasterWords, Token, Tokenizer, counts_as_word};
use tracing::{info, warn};

use crate::app::AppState;
use crate::db;
use crate::error::AppError;
use crate::stats;

const WATERMARK_KEY: &str = "tokenized_through_line_id";
/// Sessions get their own watermark: a separate id space, and a session logged
/// today can carry text read long before the newest line.
const SESSION_WATERMARK_KEY: &str = "tokenized_through_session_id";
/// The ledger's own pair, for the reason in the module doc.
const VOCAB_LINE_WATERMARK_KEY: &str = "vocab_through_line_id";
const VOCAB_SESSION_WATERMARK_KEY: &str = "vocab_through_session_id";
/// And the per-work sink's, which arrived after both.
const WORKS_LINE_WATERMARK_KEY: &str = "work_terms_through_line_id";
const WORKS_SESSION_WATERMARK_KEY: &str = "work_terms_through_session_id";
/// And the spelling sink's, latest of all.
const SURFACES_LINE_WATERMARK_KEY: &str = "term_surfaces_through_line_id";
const SURFACES_SESSION_WATERMARK_KEY: &str = "term_surfaces_through_session_id";

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
    /// Distinct `(term, work)` pairs written to `work_terms`.
    pub work_terms: usize,
    /// Distinct `(term, surface)` pairs written to `term_surfaces`.
    pub surfaces: usize,
}

impl IngestOutcome {
    fn none() -> Self {
        IngestOutcome {
            lines: 0,
            words: 0,
            terms: 0,
            work_terms: 0,
            surfaces: 0,
        }
    }
}

/// All three sinks, accumulated over one tokenization pass — kept together so
/// the tokenizer, whose dictionary load costs more than the tokenizing, runs
/// once.
struct Harvest {
    /// The master dictionary, for the affix half of the wordhood gate
    /// ([`jp_core::tokenize::counts_as_word`]). Held here because all three
    /// sinks take the same tokens and must agree about which are words.
    master: MasterWords,
    /// Every dictionary, for the other half: a short kana string none of them
    /// has is tokenizer noise and gets no row at all
    /// ([`Wordhood::is_noise`](jp_core::highlight::Wordhood::is_noise)).
    wordhood: Wordhood,
    /// `(lemma, day) → count`
    days: HashMap<(String, String), i64>,
    /// `term → (pos, count, first_ts, last_ts)`
    terms: HashMap<Term, (Option<String>, i64, f64, f64)>,
    /// `term → (occurrences Sudachi called a proper noun, occurrences seen)`,
    /// counted independently of which sinks are behind so the verdict never
    /// depends on a watermark. See [`Harvest::is_name`].
    proper: HashMap<Term, (i64, i64)>,
    /// `(term, work) → count`
    works: HashMap<(Term, String), i64>,
    /// `(term, surface) → (count, first line id)`
    surfaces: HashMap<(Term, String), (i64, Option<i64>)>,
}

impl Harvest {
    fn new(master: MasterWords, wordhood: Wordhood) -> Harvest {
        Harvest {
            master,
            wordhood,
            days: HashMap::new(),
            terms: HashMap::new(),
            proper: HashMap::new(),
            works: HashMap::new(),
            surfaces: HashMap::new(),
        }
    }
}

/// Which sinks a piece of text is behind on, and what work it belongs to.
///
/// A struct rather than three positional bools, because a swapped pair would
/// write to the wrong sink and no test would catch it — the sinks take the same
/// tokens.
#[derive(Clone, Copy)]
struct Sinks<'a> {
    days: bool,
    terms: bool,
    works: bool,
    surfaces: bool,
    work: Option<&'a str>,
    /// The line the token came from, for `term_surfaces`' example. `None` for
    /// session text, which has no per-line ids.
    line_id: Option<i64>,
}

impl Harvest {
    /// Fold one token in, into whichever sinks this piece of text is behind on.
    fn add(&mut self, t: Token, date: &str, ts: f64, sinks: Sinks<'_>) {
        if !counts_as_word(&t, &self.master) {
            return;
        }
        let term = Term::new(t.base_form, &t.reading);
        // **Before `word_days`, not after it.** A name stays in that sink
        // because it asks what text you were exposed to and a name is a word of
        // a kind; ズチュ is not a word of any kind, and counting it inflates
        // every coverage figure derived from there.
        if self.wordhood.is_noise(&term) {
            return;
        }
        if sinks.days {
            *self
                .days
                .entry((term.headword.clone(), date.to_string()))
                .or_default() += 1;
        }
        if !sinks.terms && !sinks.works && !sinks.surfaces {
            return;
        }
        let seen = self.proper.entry(term.clone()).or_default();
        seen.0 += i64::from(t.proper_noun);
        seen.1 += 1;
        if sinks.terms {
            let entry = self
                .terms
                .entry(term.clone())
                .or_insert((Some(t.pos), 0, ts, ts));
            entry.1 += 1;
            entry.2 = entry.2.min(ts);
            entry.3 = entry.3.max(ts);
        }
        if sinks.surfaces {
            let entry = self
                .surfaces
                .entry((term.clone(), t.surface))
                .or_insert((0, sinks.line_id));
            entry.0 += 1;
            entry.1 = entry.1.or(sinks.line_id);
        }
        // Unlabeled text has no work to credit. It is dropped from this sink
        // rather than bucketed under a placeholder, so `work_terms` only ever
        // claims what it can actually attribute — which is why its totals sit
        // below the ledger's.
        if let (true, Some(work)) = (sinks.works, sinks.work) {
            *self.works.entry((term, work.to_string())).or_default() += 1;
        }
    }

    /// Whether a term is a name, decided over the whole pass by majority
    /// rather than per occurrence.
    ///
    /// A name is not vocabulary — a VN's cast topped every per-work "unknown
    /// words" list. But Sudachi tags a surface 固有名詞 only some of the time,
    /// so filtering occurrence by occurrence kept 79 of ノア's 194. A majority
    /// drops ノア whole while keeping words merely *usable* as names: 空 and 光
    /// are tagged once in a hundred sightings and stay vocabulary.
    ///
    /// Names leave the ledger and the per-work sink but stay in `word_days`,
    /// which asks what text you were exposed to.
    fn is_name(&self, term: &Term) -> bool {
        let (proper, total) = self.proper.get(term).copied().unwrap_or((0, 0));
        proper * 2 > total
    }

    fn day_rows(&self) -> Vec<(String, String, i64)> {
        self.days
            .iter()
            .map(|((lemma, date), count)| (lemma.clone(), date.clone(), *count))
            .collect()
    }

    fn work_encounters(&self) -> Vec<WorkEncounter> {
        self.works
            .iter()
            .filter(|((term, _), _)| !self.is_name(term))
            .map(|((term, work), count)| WorkEncounter {
                term: term.clone(),
                work: work.clone(),
                count: *count,
            })
            .collect()
    }

    fn surface_encounters(&self) -> Vec<SurfaceEncounter> {
        self.surfaces
            .iter()
            .filter(|((term, _), _)| !self.is_name(term))
            .map(|((term, surface), (count, line_id))| SurfaceEncounter {
                term: term.clone(),
                surface: surface.clone(),
                count: *count,
                line_id: *line_id,
            })
            .collect()
    }

    fn encounters(&self) -> Vec<Encounter> {
        self.terms
            .iter()
            .filter(|(term, _)| !self.is_name(term))
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

/// Each spelling as the ingest would key it — the resolved ledger key.
///
/// The bridge between a deck or a popup and the ledger: a card is spelt the way
/// the text spelt it, Yomitan sends what the text spelt, and everything derived
/// from reading is keyed on the tokenizer's answer. Anything joining the two has
/// to resolve first or lose 検死 to 検屍.
///
/// Goes through the shared `Highlighter`, which is the reader's own tokenizer
/// with all five of its inputs, **not** a fresh `SudachiTokenizer`: see
/// [`crate::routes::reader::highlight::Highlighter::ledger_key`]. A key
/// resolved by a second pipeline matches nothing, which is the failure this is
/// here to repair.
pub(crate) async fn normalized_spellings(
    state: &AppState,
    spellings: Vec<String>,
) -> Result<Vec<String>, AppError> {
    let Some(h) = crate::routes::reader::highlight::shared(state).await else {
        return Err(AppError::Upstream(
            "tokenizer unavailable — check the Sudachi dictionary path".into(),
        ));
    };
    // Thousands of short strings on the first pass; tokenizing is CPU-bound.
    tokio::task::spawn_blocking(move || spellings.iter().map(|s| h.ledger_key(s)).collect())
        .await
        .map_err(|e| AppError::Upstream(format!("tokenize task panicked: {e}")))
}

/// Write both sinks and advance both watermarks. `max_id` is the highest id in
/// the batch; each watermark only moves for the sink that was actually behind.
async fn commit(
    state: &AppState,
    harvest: &Harvest,
    max_id: i64,
    day_key: &str,
    vocab_key: &str,
    works_key: &str,
    surfaces_key: &str,
) -> Result<(usize, usize, usize, usize), AppError> {
    let day_rows = harvest.day_rows();
    let encounters = harvest.encounters();
    let work_encounters = harvest.work_encounters();
    let surfaces = harvest.surface_encounters();
    db::add_word_day_counts(&state.knowledge, &day_rows).await?;
    jp_core::knowledge::vocabulary::record_encounters(&state.knowledge, &encounters).await?;
    jp_core::knowledge::work_terms::record_work_terms(&state.knowledge, &work_encounters).await?;
    jp_core::knowledge::term_surfaces::record_surfaces(&state.knowledge, &surfaces).await?;
    db::save_setting(&state.local, day_key, &max_id.to_string()).await?;
    db::save_setting(&state.local, vocab_key, &max_id.to_string()).await?;
    db::save_setting(&state.local, works_key, &max_id.to_string()).await?;
    db::save_setting(&state.local, surfaces_key, &max_id.to_string()).await?;
    Ok((
        day_rows.len(),
        encounters.len(),
        work_encounters.len(),
        surfaces.len(),
    ))
}

pub async fn ingest_new_lines(state: &AppState) -> Result<IngestOutcome, AppError> {
    let day_mark = watermark(state, WATERMARK_KEY).await?;
    let vocab_mark = watermark(state, VOCAB_LINE_WATERMARK_KEY).await?;
    let works_mark = watermark(state, WORKS_LINE_WATERMARK_KEY).await?;
    let surfaces_mark = watermark(state, SURFACES_LINE_WATERMARK_KEY).await?;

    // Re-read from whichever sink is furthest behind; each token is then
    // credited only to the sinks that have not had it.
    let behind = day_mark.min(vocab_mark).min(works_mark).min(surfaces_mark);
    let lines = db::fetch_lines_after(&state.knowledge, behind).await?;
    let Some(max_id) = lines.last().map(|l| l.id) else {
        return Ok(IngestOutcome::none());
    };

    let settings = db::load_settings(&state.local).await?;
    let rollover = settings.day_rollover_hour;
    let tz = tz_offset_secs();
    // The same seven inputs the reader's `Highlighter` takes — one call, so a
    // tint and the ledger row behind it cannot come from two pipelines.
    let p = jp_core::highlight::pipeline(&state.knowledge, &state.sudachi_dict_path)
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?;

    let n_lines = lines.len();
    // Dictionary load + tokenization are CPU-bound; keep them off the runtime.
    let harvest = tokio::task::spawn_blocking(move || -> Result<_, AppError> {
        let tokenizer = p.tokenizer;
        let mut harvest = Harvest::new(p.master, p.wordhood);
        for line in &lines {
            let date = stats::date_key(line.ts, rollover, tz).to_string();
            let sinks = Sinks {
                days: line.id > day_mark,
                terms: line.id > vocab_mark,
                works: line.id > works_mark,
                surfaces: line.id > surfaces_mark,
                work: line.work.as_deref(),
                line_id: Some(line.id),
            };
            match tokenizer.tokenize(&line.text) {
                Ok(tokens) => {
                    for t in tokens {
                        harvest.add(t, &date, line.ts, sinks);
                    }
                }
                Err(e) => warn!(line_id = line.id, error = %e, "tokenize failed, skipping line"),
            }
        }
        Ok(harvest)
    })
    .await
    .map_err(|e| AppError::Upstream(format!("tokenize task panicked: {e}")))??;

    let (words, terms, work_terms, surfaces) = commit(
        state,
        &harvest,
        max_id,
        WATERMARK_KEY,
        VOCAB_LINE_WATERMARK_KEY,
        WORKS_LINE_WATERMARK_KEY,
        SURFACES_LINE_WATERMARK_KEY,
    )
    .await?;

    info!(
        lines = n_lines,
        words, terms, work_terms, surfaces, "line ingest complete"
    );
    Ok(IngestOutcome {
        lines: n_lines,
        words,
        terms,
        work_terms,
        surfaces,
    })
}

/// The same pass over manually logged sessions that carry their text.
///
/// A session's words all land on the day its `start_ts` falls in: there are no
/// per-line timestamps to spread them over.
///
/// Reading an article genuinely re-shows you a word, so these counts belong in
/// `word_days` and the ledger beside the hooked ones. Deliberately *not* true of
/// the lookup rates, which stay over hooked reading only (`stats::rate`).
pub async fn ingest_new_sessions(state: &AppState) -> Result<IngestOutcome, AppError> {
    let day_mark = watermark(state, SESSION_WATERMARK_KEY).await?;
    let vocab_mark = watermark(state, VOCAB_SESSION_WATERMARK_KEY).await?;
    let works_mark = watermark(state, WORKS_SESSION_WATERMARK_KEY).await?;
    let surfaces_mark = watermark(state, SURFACES_SESSION_WATERMARK_KEY).await?;

    let behind = day_mark.min(vocab_mark).min(works_mark).min(surfaces_mark);
    let sessions = db::fetch_session_texts_after(&state.knowledge, behind).await?;
    let Some(max_id) = sessions.last().map(|s| s.id) else {
        return Ok(IngestOutcome::none());
    };

    let settings = db::load_settings(&state.local).await?;
    let rollover = settings.day_rollover_hour;
    let tz = tz_offset_secs();
    // The same seven inputs the reader's `Highlighter` takes — one call, so a
    // tint and the ledger row behind it cannot come from two pipelines.
    let p = jp_core::highlight::pipeline(&state.knowledge, &state.sudachi_dict_path)
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?;

    let n_sessions = sessions.len();
    let harvest = tokio::task::spawn_blocking(move || -> Result<_, AppError> {
        let tokenizer = p.tokenizer;
        let mut harvest = Harvest::new(p.master, p.wordhood);
        for s in &sessions {
            let date = stats::date_key(s.start_ts, rollover, tz).to_string();
            // Articles collapse under one synthetic work here exactly as they
            // do on the shelf, so a logged article's words are attributable
            // without thirty one-row works appearing beside the VNs.
            let work = stats::work_key(&s.source, s.work.as_deref());
            let sinks = Sinks {
                days: s.id > day_mark,
                terms: s.id > vocab_mark,
                works: s.id > works_mark,
                surfaces: s.id > surfaces_mark,
                work: work.as_deref(),
                line_id: None,
            };
            // Sentence by sentence: Sudachi has an input length limit, and an
            // article is far longer than the hooked line this path was built
            // for. Splitting keeps every token in the same analysis window it
            // would have had as a line.
            for sentence in jp_core::text::sentences::split_sentences(&s.content) {
                match tokenizer.tokenize(&sentence) {
                    Ok(tokens) => {
                        for t in tokens {
                            harvest.add(t, &date, s.start_ts, sinks);
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

    let (words, terms, work_terms, surfaces) = commit(
        state,
        &harvest,
        max_id,
        SESSION_WATERMARK_KEY,
        VOCAB_SESSION_WATERMARK_KEY,
        WORKS_SESSION_WATERMARK_KEY,
        SURFACES_SESSION_WATERMARK_KEY,
    )
    .await?;

    info!(
        sessions = n_sessions,
        words, terms, work_terms, surfaces, "session ingest complete"
    );
    Ok(IngestOutcome {
        lines: n_sessions,
        words,
        terms,
        work_terms,
        surfaces,
    })
}

/// Rewind the ledger's watermarks so the next ingest re-reads the whole history
/// into `vocabulary`, without touching `word_days`.
///
/// Safe to run more than once: rows are recreated from scratch rather than added
/// to, so a second run gives the same numbers. `status` survives — the reset
/// clears counts, and an assertion is not a count.
pub async fn reset_vocabulary(state: &AppState) -> Result<(), AppError> {
    sqlx::query("UPDATE vocabulary SET encounter_count = 0, first_seen = NULL, last_seen = NULL")
        .execute(state.knowledge.pool())
        .await?;
    // The per-work sink is rebuilt from the same pass, so it rewinds with the
    // ledger. Deleted rather than zeroed: a `(term, work)` pair that no longer
    // occurs should disappear, and unlike `vocabulary` these rows carry
    // nothing a reader asserted.
    jp_core::knowledge::work_terms::reset(&state.knowledge).await?;
    // Same reasoning: derived wholly from the pass, and carries no assertion.
    jp_core::knowledge::term_surfaces::reset(&state.knowledge).await?;
    db::save_setting(&state.local, VOCAB_LINE_WATERMARK_KEY, "0").await?;
    db::save_setting(&state.local, VOCAB_SESSION_WATERMARK_KEY, "0").await?;
    db::save_setting(&state.local, WORKS_LINE_WATERMARK_KEY, "0").await?;
    db::save_setting(&state.local, WORKS_SESSION_WATERMARK_KEY, "0").await?;
    db::save_setting(&state.local, SURFACES_LINE_WATERMARK_KEY, "0").await?;
    db::save_setting(&state.local, SURFACES_SESSION_WATERMARK_KEY, "0").await?;
    info!("vocabulary ledger counts reset; next ingest will rebuild them");
    Ok(())
}

/// Resolve every lookup spelling Yomitan has sent since the last pass to the
/// ledger key it means.
///
/// Here rather than in `ankiproxy::record`, which writes the row: that runs on
/// the mining hot path, where nothing may be awaited in front of the capture,
/// and it would pay a Sudachi dictionary load per popup. `lookup_count` is
/// recomputed wholesale on the Anki refresh, so resolving just before it is as
/// timely as writing it at the popup would have been.
///
/// Only unresolved rows, so this is a no-op on a refresh with no new lookups
/// and the first pass is the only one that pays for the backlog.
async fn normalize_new_lookups(state: &AppState) -> Result<u64, AppError> {
    let terms = db::unnormalized_lookup_terms(&state.knowledge).await?;
    if terms.is_empty() {
        return Ok(0);
    }
    let headwords = normalized_spellings(state, terms.clone()).await?;
    let resolved: Vec<(String, String)> = terms.into_iter().zip(headwords).collect();
    let rows = db::set_lookup_headwords(&state.knowledge, &resolved).await?;
    info!(
        terms = resolved.len(),
        rows, "resolved lookup spellings to ledger keys"
    );
    Ok(rows)
}

/// The wholesale syncs, run after an ingest: Anki owns `mined`, `lookups` owns
/// `lookup_count`, the dictionaries own the wordhood flags. None of the three
/// touches `status`.
pub async fn sync_vocabulary(state: &AppState) -> Result<i64, AppError> {
    let mined = jp_core::knowledge::vocabulary::sync_mined(&state.knowledge).await?;
    // Before the count that reads it, never after.
    normalize_new_lookups(state).await?;
    jp_core::knowledge::vocabulary::sync_lookup_counts(&state.knowledge).await?;
    jp_core::knowledge::vocabulary::refresh_dictionary_flags(&state.knowledge).await?;
    Ok(mined)
}

/// Fold a whole extracted script into per-work term counts.
///
/// Goes through `Harvest` and so through the same wordhood gate and the same
/// name-majority rule reading does, because the point of the result is to be
/// compared with the ledger: a count derived by a second pipeline would answer
/// a different question while looking like this one.
///
/// Only the per-work sink is filled. A script is text that exists, not text
/// that was read, so it must not touch `word_days`, the ledger, or the
/// spellings sink — nothing here is an encounter.
pub async fn profile_script(
    knowledge: &jp_core::knowledge::Knowledge,
    dict_path: &std::path::Path,
    work: &str,
    lines: Vec<String>,
) -> Result<(Vec<WorkEncounter>, i64), AppError> {
    let p = jp_core::highlight::pipeline(knowledge, dict_path)
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?;

    let work = work.to_string();
    let encounters = tokio::task::spawn_blocking(move || {
        let tokenizer = p.tokenizer;
        let mut harvest = Harvest::new(p.master, p.wordhood);
        let sinks = Sinks {
            days: false,
            terms: false,
            works: true,
            surfaces: false,
            work: Some(&work),
            line_id: None,
        };
        for line in &lines {
            match tokenizer.tokenize(line) {
                Ok(tokens) => {
                    for t in tokens {
                        harvest.add(t, "", 0.0, sinks);
                    }
                }
                Err(e) => warn!(error = %e, "tokenize failed, skipping line"),
            }
        }
        harvest.work_encounters()
    })
    .await
    .map_err(|e| AppError::Upstream(format!("tokenize task panicked: {e}")))?;

    let total = encounters.iter().map(|e| e.count).sum();
    Ok((encounters, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(base: &str, reading: &str, proper: bool) -> Token {
        Token {
            surface: base.to_string(),
            base_form: base.to_string(),
            dictionary_form: base.to_string(),
            reading: reading.to_string(),
            pos: "名詞".to_string(),
            proper_noun: proper,
            subsidiary: false,
            counter: false,
            derived_class: None,
            inflected: false,
        }
    }

    fn all_sinks() -> Sinks<'static> {
        Sinks {
            days: true,
            terms: true,
            works: true,
            surfaces: true,
            work: Some("A"),
            line_id: Some(1),
        }
    }

    /// A master dictionary for the tests: 達/たち, so the affix rule has
    /// something to admit, and nothing else.
    fn test_master() -> MasterWords {
        MasterWords::new(
            ["達".to_string()].into_iter().collect(),
            &[("達".to_string(), "たち".to_string())],
        )
    }

    fn harvest(tokens: Vec<Token>) -> Harvest {
        harvest_with(Wordhood::default(), tokens)
    }

    fn harvest_with(wordhood: Wordhood, tokens: Vec<Token>) -> Harvest {
        let mut h = Harvest::new(test_master(), wordhood);
        for t in tokens {
            h.add(t, "2026-07-27", 0.0, all_sinks());
        }
        h
    }

    /// Tokenizer noise reaches no sink at all — not even `word_days`, which a
    /// name does reach. ズチュ is not a word of any kind, and counting it
    /// inflates every coverage figure derived from there.
    #[test]
    fn short_kana_no_dictionary_has_reaches_no_sink() {
        let listed = Wordhood::new(
            ["達".to_string()].into_iter().collect(),
            ["たち".to_string()].into_iter().collect(),
        );
        let h = harvest_with(listed, vec![tok("ズチュ", "ズチュ", false); 12]);
        assert!(h.encounters().is_empty());
        assert!(h.work_encounters().is_empty());
        assert!(h.day_rows().is_empty(), "not exposure to a word either");
    }

    #[test]
    fn a_name_never_reaches_the_ledger_or_the_per_work_sink() {
        let h = harvest(vec![tok("ノア", "のあ", true); 20]);
        assert!(h.encounters().is_empty());
        assert!(h.work_encounters().is_empty());
        // Exposure still counts it: `word_days` asks what text was read, and a
        // name on the page is text.
        assert_eq!(h.day_rows().len(), 1);
    }

    #[test]
    fn a_word_that_is_occasionally_tagged_a_name_stays_vocabulary() {
        // 空 is そら nearly always and a surname once in a while. Filtering per
        // occurrence would have kept a fraction of its count, which is a worse
        // answer than either whole one.
        let mut tokens = vec![tok("空", "そら", false); 30];
        tokens.push(tok("空", "そら", true));
        let h = harvest(tokens);
        assert_eq!(h.encounters().len(), 1);
        assert_eq!(h.encounters()[0].count, 31, "every occurrence counts");
    }

    #[test]
    fn the_verdict_is_the_majority_of_a_terms_own_occurrences() {
        // Sudachi tags a VN's cast inconsistently — this one only 60% of the
        // time, which is still a name.
        let mut tokens = vec![tok("ノア", "のあ", true); 6];
        tokens.extend(vec![tok("ノア", "のあ", false); 4]);
        assert!(harvest(tokens).encounters().is_empty());
    }

    #[test]
    fn a_suffix_the_master_lists_reaches_the_ledger() {
        // 私達 is not a master entry, so it arrives as 私 + 達. Dropping the
        // suffix half credited the compound's second word to nothing — the
        // 懲罰房 defect, arriving through the part-of-speech tag instead.
        let mut suffix = tok("達", "たち", false);
        suffix.pos = "接尾辞".to_string();
        let h = harvest(vec![suffix; 3]);
        assert_eq!(h.encounters().len(), 1);
        assert_eq!(h.encounters()[0].count, 3);
    }

    #[test]
    fn a_suffix_no_dictionary_lists_is_still_dropped() {
        // げ, ぷ, さん/さーん — tokenizer output with nothing behind it.
        let mut noise = tok("げ", "げ", false);
        noise.pos = "接尾辞".to_string();
        let h = harvest(vec![noise; 9]);
        assert!(h.encounters().is_empty());
        assert!(h.day_rows().is_empty());
    }

    #[test]
    fn the_spelling_sink_keeps_what_the_page_said() {
        // The ledger key is the normalized form for both of these; the whole
        // point of the sink is that they are not the same question.
        let mut kana = tok("窺う", "うかがう", false);
        kana.surface = "うかがう".to_string();
        let mut kanji = tok("窺う", "うかがう", false);
        kanji.surface = "窺っ".to_string();
        let h = harvest(vec![kana.clone(), kana, kanji]);

        assert_eq!(h.encounters().len(), 1, "one ledger row, as before");
        let mut spellings: Vec<(String, i64)> = h
            .surface_encounters()
            .iter()
            .map(|s| (s.surface.clone(), s.count))
            .collect();
        spellings.sort();
        assert_eq!(
            spellings,
            vec![("うかがう".to_string(), 2), ("窺っ".to_string(), 1)]
        );
    }

    #[test]
    fn a_name_leaves_the_spelling_sink_with_the_ledger() {
        let h = harvest(vec![tok("ノア", "のあ", true); 20]);
        assert!(h.surface_encounters().is_empty());
    }

    #[test]
    fn unlabeled_text_reaches_every_sink_but_the_per_work_one() {
        let mut h = Harvest::new(test_master(), Wordhood::default());
        h.add(
            tok("猫", "ねこ", false),
            "2026-07-27",
            0.0,
            Sinks {
                work: None,
                ..all_sinks()
            },
        );
        assert_eq!(h.encounters().len(), 1);
        assert!(
            h.work_encounters().is_empty(),
            "nothing to attribute it to, and no placeholder bucket"
        );
    }
}
