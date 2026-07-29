//! `/api/vocab/*` — the knowledge ledger's status endpoints.
//!
//! Reads, the rebuild, and the triage pass that fills `status`
//! (`spec/cold-start.md` Pass 2, over terms already in the ledger). The ledger
//! itself is `jp_core::knowledge::vocabulary`.
//!
//! The one rule these handlers exist to keep: **`status` is only ever written
//! from a request the reader made.** No sync touches it, so the ledger cannot
//! demote a word behind their back and an encounter count cannot promote one.

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, Query, State};
use jp_core::knowledge::dictionaries;
use jp_core::knowledge::lexeme;
use jp_core::knowledge::vocabulary::{self, Status, Term};
use serde::Deserialize;
use serde_json::{Value, json};

use jp_core::tokenize::{SudachiTokenizer, Tokenizer};
use tracing::info;

use crate::app::AppState;
use crate::clock::now_ts;
use crate::db;
use crate::error::AppError;

/// Rows per queue page. A batch big enough to be worth one sweep of attention
/// and small enough that submitting it is not a big commitment.
const QUEUE_LIMIT: i64 = 200;

/// Rows per page of the non-vocabulary tail. A screenful, not a sample: the
/// whole set is reachable by paging.
const NON_WORD_PAGE: i64 = 100;

/// Rows per page of frequency-triage candidates. Larger than the non-word
/// page: every homograph is filtered out before paging (`frequency_queue`),
/// so every row on screen is one the "mark known" button can actually act
/// on, and a bigger page means fewer round trips through a threshold that
/// can hold thousands of committable words.
const FREQUENCY_PAGE: i64 = 500;

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

    // `known_in_master` counts ledger *rows*; `known_words` counts words.
    // They differ by spelling alone — alternate kanji forms of one entry, and
    // kana spellings of words known in kanji — so the second is the honest
    // "I know N words" figure and the first is what fills the queue.
    let words = lexeme::known_lexemes(&state.knowledge).await?;

    // `seen` is derived, never stored: untriaged terms met often enough to be
    // worth offering. It shares the triage floor so the number and the queue
    // cannot disagree about what counts as met.
    let settings = db::load_settings(&state.local).await?;
    let seen = vocabulary::seen_count(&state.knowledge, settings.triage_min_encounters).await?;

    Ok(Json(json!({
        "total": total,
        "known_in_master": known,
        "known_words": words,
        "seen": seen,
        "seen_min_encounters": settings.triage_min_encounters,
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

/// Re-home every judgement the rebuild stranded.
///
/// A stranded row is one the reader judged and the ingest no longer produces —
/// after the move to normalized headwords, いっぱい and あげる became 一杯 and
/// 上げる. The tokenizer says what each old key is called now: if that name is
/// in the ledger, the judgement moves onto it.
///
/// The tokenizer is the authority rather than a string rule, and a row it
/// cannot resolve to a single token is left alone — a stranded judgement is
/// harmless, and a misplaced one is not.
async fn carry_stranded_judgements(state: &AppState) -> Result<usize, AppError> {
    let stranded = vocabulary::stranded_judgements(&state.knowledge).await?;
    if stranded.is_empty() {
        return Ok(0);
    }
    let dict_path = state.sudachi_dict_path.clone();
    let plan = tokio::task::spawn_blocking(move || -> Result<Vec<(Term, Term)>, AppError> {
        let tokenizer = SudachiTokenizer::new(&dict_path, Default::default())
            .map_err(|e| AppError::Upstream(format!("sudachi: {e}")))?;
        let mut plan = Vec::new();
        for row in &stranded {
            let Ok(tokens) = tokenizer.tokenize(&row.term.headword) else {
                continue;
            };
            let [t] = tokens.as_slice() else { continue };
            let now_called = Term::new(t.base_form.clone(), &t.reading);
            if now_called != row.term {
                plan.push((row.term.clone(), now_called));
            }
        }
        Ok(plan)
    })
    .await
    .map_err(|e| AppError::Upstream(format!("tokenize task panicked: {e}")))??;

    let mut carried = 0;
    for (from, into) in &plan {
        if vocabulary::carry_judgement(&state.knowledge, from, into).await? {
            carried += 1;
        }
    }
    info!(
        carried,
        "moved judgements onto the keys the ingest now writes"
    );
    Ok(carried)
}

/// What `blacklist-non-words` would blacklist, before it does.
///
/// The action is a bulk write over rows the queue never shows, so without this
/// the reader is asked to approve a predicate they have never seen the output
/// of. Same `WHERE`, commonest first, and paged rather than truncated: a
/// preview that only ever shows the head cannot answer whether the tail is
/// safe, which is the question.
pub async fn vocab_non_words(
    State(state): State<AppState>,
    Query(params): Query<PageParams>,
) -> Result<Json<Value>, AppError> {
    let limit = params.limit.unwrap_or(NON_WORD_PAGE).clamp(1, 500);
    let offset = params.offset.unwrap_or(0).max(0);
    let rows = vocabulary::non_words(&state.knowledge, limit, offset).await?;
    let total = vocabulary::non_words_total(&state.knowledge).await?;
    Ok(Json(json!({
        "total": total,
        "offset": offset,
        "limit": limit,
        "terms": rows
            .iter()
            .map(|r| json!({
                "headword": r.term.headword,
                "reading": r.term.display_reading(),
                "pos": r.pos,
                "encounter_count": r.encounter_count,
            }))
            .collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
pub struct PageParams {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
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

/// Import the Anki review pile as `known` (`spec/cold-start.md` Pass 1).
///
/// Reader-triggered only, like `vocab_judge` and `vocab_blacklist_non_words` —
/// never folded into `anki_refresh`'s recurring snapshot, which must never
/// write `status`. "Reviewing" (`-is:new -is:learn`) is the gate: a card still
/// in Anki's new/learning queues is a word explicitly not yet had, so those
/// notes are left untouched rather than imported as anything.
///
/// Anki has no reading beside the vocab field, so each term is resolved
/// against the master dictionary the same way frequency triage resolves one:
/// zero matches isn't master vocabulary (stored with an empty reading, same as
/// any kana-only term); more than one is a homograph, skipped and counted
/// rather than guessed at — few enough of those to leave for ordinary
/// encounter-based triage to sort out later.
pub async fn vocab_anki_import(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Result<Json<Value>, AppError> {
    let mut last_err = AppError::Upstream(format!(
        "no AnkiConnect reachable (tried dashboard client {} and {})",
        addr.ip(),
        state.anki_url
    ));
    let mut notes = None;
    for url in crate::services::anki::candidate_urls(Some(addr.ip()), &state.anki_url) {
        match crate::services::anki::fetch_reviewed_deck_vocab(
            &state.http,
            &url,
            &state.anki_deck,
            &state.anki_vocab_field,
        )
        .await
        {
            Ok(n) => {
                notes = Some(n);
                break;
            }
            Err(e) => last_err = e,
        }
    }
    let Some(notes) = notes else {
        return Err(last_err);
    };

    let mut judgements = Vec::with_capacity(notes.len());
    let mut ambiguous_skipped = 0i64;
    for note in &notes {
        let readings = dictionaries::master_readings(state.knowledge.pool(), &note.vocab).await?;
        match readings.as_slice() {
            [] => judgements.push((Term::new(note.vocab.clone(), ""), Status::Known)),
            [reading] => judgements.push((Term::new(note.vocab.clone(), reading), Status::Known)),
            _ => ambiguous_skipped += 1,
        }
    }

    let imported = vocabulary::set_status_each(&state.knowledge, &judgements, now_ts()).await?;
    Ok(Json(json!({
        "imported": imported,
        "ambiguous_skipped": ambiguous_skipped,
    })))
}

/// jiten.moe's JSON export: a list of cards keyed by JMdict entry id.
///
/// Only `w` is read. The status field (`s`) is deliberately ignored — every
/// card in the export is imported as `known`, by the reader's instruction:
/// jiten's own maturity grades are its scheduler's business, not an assertion
/// about what is known.
#[derive(Deserialize)]
pub struct JitenExport {
    cards: Vec<JitenCard>,
}

#[derive(Deserialize)]
pub struct JitenCard {
    /// JMdict `ent_seq`. The same id `dictionary_entries.sequence` stores.
    w: i64,
}

/// Pass 5: seed the ledger from a jiten.moe export.
///
/// The only source so far that names *words* rather than spellings. Passes 1
/// and 3 both had to infer a reading from a bare headword and skip whatever
/// came back ambiguous — 73 and 159 terms respectively. This one carries
/// JMdict entry ids, so 辛い/つらい is marked and 辛い/からい is not, without
/// anything being guessed. There is no ambiguous-skipped count here because
/// there is no ambiguity.
///
/// An id fans out to every spelling of it the **master** dictionary lists, and
/// that fan-out is safe only because counting collapses back:
/// `jp_core::knowledge::lexeme` reports こちら and こっち as one word, so
/// marking both cannot inflate the vocabulary figure. What the fan-out buys is
/// that triage stops asking about each spelling separately.
///
/// Import order does not matter and re-running is free: `status` is set, never
/// cleared, and counts are left alone (`set_status_each`).
pub async fn vocab_jiten_import(
    State(state): State<AppState>,
    Json(export): Json<JitenExport>,
) -> Result<Json<Value>, AppError> {
    let forms_by_seq = dictionaries::master_forms_by_sequence(state.knowledge.pool()).await?;

    let ids: std::collections::HashSet<i64> = export.cards.iter().map(|c| c.w).collect();
    let mut terms: std::collections::HashSet<Term> = std::collections::HashSet::new();
    let mut unresolved = 0i64;
    for id in &ids {
        match forms_by_seq.get(id) {
            // No master spelling: a name, or one of JMdict's phrase entries.
            // Not vocabulary, so it imports as nothing rather than as a row
            // the triage queue would then have to reject.
            None => unresolved += 1,
            Some(forms) => {
                for (headword, reading) in forms {
                    terms.insert(Term::new(headword.clone(), reading));
                }
            }
        }
    }

    let judgements: Vec<(Term, Status)> = terms.into_iter().map(|t| (t, Status::Known)).collect();
    let marked = vocabulary::seed_status_each(&state.knowledge, &judgements, now_ts()).await?;

    // Most of what this just wrote are rows that did not exist a moment ago —
    // words never met in any reading — and a fresh row's dictionary flags are
    // all zero until something fills them. Without this the vocabulary scale
    // ignores the entire import: `in_master` is what the count gates on, and
    // 6,255 freshly seeded words sat outside it.
    vocabulary::refresh_dictionary_flags(&state.knowledge).await?;

    info!(
        cards = export.cards.len(),
        ids = ids.len(),
        resolved = ids.len() as i64 - unresolved,
        unresolved,
        marked,
        "jiten import"
    );
    Ok(Json(json!({
        "cards": export.cards.len(),
        "ids": ids.len(),
        "resolved_entries": ids.len() as i64 - unresolved,
        "unresolved_entries": unresolved,
        "terms_marked": marked,
    })))
}

/// How many encounters make a non-master term worth asking about, when no
/// card exists for it. Low, because the two sources are asymmetric: a mined
/// term is already the reader's own claim, while a read one has only the
/// tokenizer's word for it, and the queue would fill with noise at 1.
const PROMOTION_MIN_ENCOUNTERS: i64 = 5;

/// Rows per page of promotion candidates. Small — this is a judgement per
/// row, not a sweep.
const PROMOTION_PAGE: i64 = 200;

/// The escape hatch's queue: terms the master dictionary does not list, but
/// which the reader has either mined or read repeatedly.
///
/// Sankoku carries no 冪等性, no 可用性, and no stem of either, so no
/// decomposition rule reaches them; JMdict would admit them along with every
/// idiom and orthographic variant. So the reader decides, one term at a time —
/// and mining is what surfaces the term, since every reading-based queue gates
/// on the vocabulary predicate and can never offer a non-master word.
pub async fn vocab_promotion_queue(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let rows = vocabulary::promotion_candidates(
        &state.knowledge,
        PROMOTION_MIN_ENCOUNTERS,
        PROMOTION_PAGE,
    )
    .await?;
    let pending = vocabulary::promotion_pending(&state.knowledge, PROMOTION_MIN_ENCOUNTERS).await?;
    let terms: Vec<Value> = rows
        .iter()
        .map(|c| {
            json!({
                "headword": c.term.headword,
                "reading": c.term.display_reading(),
                "mined": c.mined,
                "encounter_count": c.encounter_count,
                "in_reference": c.in_reference,
            })
        })
        .collect();
    Ok(Json(json!({
        "terms": terms,
        "pending": pending,
        "min_encounters": PROMOTION_MIN_ENCOUNTERS,
    })))
}

#[derive(Deserialize)]
pub struct PromoteRequest {
    terms: Vec<TermRef>,
    /// Absent means promote. Sent explicitly to undo one.
    #[serde(default = "yes")]
    promoted: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
pub struct TermRef {
    headword: String,
    #[serde(default)]
    reading: String,
}

/// Count these terms as vocabulary, or stop counting them.
///
/// Never touches `status`: promoting 冪等性 asserts it is a word, not that it
/// is known, and for anything still in this queue those have different
/// answers.
pub async fn vocab_promote(
    State(state): State<AppState>,
    Json(req): Json<PromoteRequest>,
) -> Result<Json<Value>, AppError> {
    let terms: Vec<Term> = req
        .terms
        .iter()
        .map(|t| Term::new(t.headword.clone(), &t.reading))
        .collect();
    let changed = vocabulary::set_promoted(&state.knowledge, &terms, req.promoted).await?;
    info!(changed, promoted = req.promoted, "vocabulary promotion");
    Ok(Json(json!({ "changed": changed })))
}

/// Repair the empty-reading rows Pass 1 created for kanji headwords.
///
/// Reader-triggered rather than automatic: it merges and deletes rows, and a
/// destructive repair should happen when someone asked for it. Idempotent, so
/// running it twice is harmless.
pub async fn vocab_repair_empty_readings(
    State(state): State<AppState>,
) -> Result<Json<Value>, AppError> {
    let r = vocabulary::repair_empty_readings(&state.knowledge).await?;
    vocabulary::refresh_dictionary_flags(&state.knowledge).await?;
    info!(rekeyed = r.rekeyed, merged = r.merged, unresolved = r.unresolved, "reading repair");
    Ok(Json(json!({
        "rekeyed": r.rekeyed,
        "merged": r.merged,
        "unresolved": r.unresolved,
    })))
}

/// How many BCCWJ terms at or under a rank threshold are unjudged master
/// vocabulary — the cheap count a threshold slider previews against before
/// paging through the actual list.
pub async fn vocab_frequency_summary(
    State(state): State<AppState>,
    Query(params): Query<FrequencyParams>,
) -> Result<Json<Value>, AppError> {
    let settings = db::load_settings(&state.local).await?;
    let max_rank = params.max_rank.unwrap_or(settings.triage_max_freq_rank).max(1);
    let (bccwj, master) = frequency_dictionaries(&state).await?;
    let pending = vocabulary::frequency_pending(&state.knowledge, bccwj, master, max_rank).await?;
    Ok(Json(json!({
        "max_rank": max_rank,
        "committable": pending.committable,
        "ambiguous": pending.ambiguous,
    })))
}

#[derive(Deserialize)]
pub struct FrequencyParams {
    max_rank: Option<i64>,
    limit: Option<i64>,
    offset: Option<i64>,
}

/// A page of frequency-triage candidates — the preview `frequency-commit`
/// would act on, shown before it does (same rule as `vocab_non_words`).
///
/// Homographs are never on this list — `vocabulary::frequency_queue` excludes
/// them at the query, not just at display, so every row here is one the
/// commit button can actually write and paging never spends a slot on a row
/// the reader has to skip past for nothing. `ambiguous` is still reported, as
/// a total from `frequency_pending`, for the one line that says how many
/// words are being left out and why.
pub async fn vocab_frequency_queue(
    State(state): State<AppState>,
    Query(params): Query<FrequencyParams>,
) -> Result<Json<Value>, AppError> {
    let settings = db::load_settings(&state.local).await?;
    let max_rank = params.max_rank.unwrap_or(settings.triage_max_freq_rank).max(1);
    let limit = params.limit.unwrap_or(FREQUENCY_PAGE).clamp(1, 2000);
    let offset = params.offset.unwrap_or(0).max(0);
    let (bccwj, master) = frequency_dictionaries(&state).await?;

    let pending = vocabulary::frequency_pending(&state.knowledge, bccwj, master, max_rank).await?;
    let rows =
        vocabulary::frequency_queue(&state.knowledge, bccwj, master, max_rank, limit, offset)
            .await?;
    let terms: Vec<Value> = rows
        .iter()
        .map(|r| json!({ "term": r.term, "rank": r.rank, "reading": r.reading }))
        .collect();

    Ok(Json(json!({
        "max_rank": max_rank,
        "total": pending.committable,
        "ambiguous": pending.ambiguous,
        "offset": offset,
        "limit": limit,
        "terms": terms,
    })))
}

#[derive(Deserialize)]
pub struct FrequencyCommitRequest {
    max_rank: i64,
}

/// Mark every committable term at or under `max_rank` `known`, in one sweep.
///
/// The bulk-commit half of Pass 3: a threshold and a click, not a swipe per
/// word. Persists the threshold into `triage_max_freq_rank` so the next visit
/// remembers it, same as the triage floor does.
///
/// `frequency_queue` already excludes homographs and already-judged rows
/// (`FREQUENCY_FILTER`'s `status != 'new'` guard — which is also why a word
/// the reader clicked "not known" on while previewing does not come back
/// here: `vocab_judge` already gave it a status), so there is nothing left to
/// resolve or skip at this layer. `ambiguous_skipped` comes from
/// `frequency_pending` rather than being counted here, since it names the
/// same set either way and a second count could only drift from it.
pub async fn vocab_frequency_commit(
    State(state): State<AppState>,
    Json(req): Json<FrequencyCommitRequest>,
) -> Result<Json<Value>, AppError> {
    let max_rank = req.max_rank.max(1);
    let (bccwj, master) = frequency_dictionaries(&state).await?;

    let pending = vocabulary::frequency_pending(&state.knowledge, bccwj, master, max_rank).await?;
    // Unbounded (SQLite's own convention: LIMIT -1 means no limit), at this
    // scale a few thousand rows even at a generous threshold.
    let rows = vocabulary::frequency_queue(&state.knowledge, bccwj, master, max_rank, -1, 0).await?;
    let judgements: Vec<(Term, Status)> = rows
        .iter()
        .map(|r| (Term::new(r.term.clone(), &r.reading), Status::Known))
        .collect();

    let written = vocabulary::set_status_each(&state.knowledge, &judgements, now_ts()).await?;
    db::save_setting(&state.local, "triage_max_freq_rank", &max_rank.to_string()).await?;
    Ok(Json(json!({ "written": written, "ambiguous_skipped": pending.ambiguous })))
}

/// The two dictionary ids frequency triage joins against — BCCWJ (by title,
/// since it carries no distinguishing role) and the master dictionary.
/// Neither existing is a configuration problem, not a request one: both are
/// loaded once at startup, so a missing one means the wrong deployment rather
/// than a retryable condition.
async fn frequency_dictionaries(state: &AppState) -> Result<(i64, i64), AppError> {
    let bccwj = dictionaries::by_title(state.knowledge.pool(), "BCCWJ")
        .await?
        .ok_or_else(|| AppError::Upstream("BCCWJ frequency dictionary not loaded".into()))?;
    let master = dictionaries::master(state.knowledge.pool())
        .await?
        .ok_or_else(|| AppError::Upstream("no master dictionary set".into()))?;
    Ok((bccwj.id, master.id))
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
    // A re-tokenization moves words between keys, and an assertion left on the
    // old one is a judgement the reader made about a word that now lives
    // elsewhere. Carry it before pruning, or the next step deletes the answer.
    let carried = carry_stranded_judgements(&state).await?;
    // Anything the re-ingest did not touch is no longer in the reading — a
    // proper noun now that names are excluded, or a term the tokenizer splits
    // differently than it used to. Judged rows and mined rows are spared.
    let pruned = vocabulary::prune_untouched(&state.knowledge).await?;

    Ok(Json(json!({
        "lines": lines,
        "sessions": sessions,
        "mined_terms": mined,
        "carried": carried,
        "pruned": pruned,
    })))
}
