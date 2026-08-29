//! `/api/vocab/*` — the knowledge ledger's status endpoints. The ledger itself
//! is `jp_core::knowledge::vocabulary`.
//!
//! The rule these handlers exist to keep: **`status` is only ever written from a
//! request the reader made.** No sync touches it, so the ledger cannot demote a
//! word behind their back and an encounter count cannot promote one.

use std::collections::HashMap;
use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, Query, State};
use jp_core::knowledge::dictionaries;
use jp_core::knowledge::lexeme;
use jp_core::knowledge::term_surfaces;
use jp_core::knowledge::vocabulary::{self, Status, Term};
use serde::Deserialize;
use serde_json::{Value, json};

use jp_core::tokenize::{SudachiTokenizer, Tokenizer};
use tracing::info;

use crate::app::AppState;
use crate::clock::{now_ts, tz_offset_secs};
use crate::db;
use crate::error::AppError;
use crate::stats;

const QUEUE_LIMIT: i64 = 200;

/// The whole non-vocabulary tail is reachable by paging.
const NON_WORD_PAGE: i64 = 100;

/// What the ledger holds, by status.
///
/// `in_master` is the vocabulary scale: a term counts toward "I know N words"
/// only if the master dictionary lists it. Jitendex is a phrase index and would
/// make the number meaningless.
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

    // `known_in_master` counts ledger *rows*, `known_words` counts words —
    // they differ by spelling alone, so the second is the honest "I know N
    // words" figure and the first is what fills the queue.
    let words = lexeme::known_lexemes(&state.knowledge).await?;

    // `status = 'new'` means "never judged", which says nothing about whether
    // the word was met, so the bucket is split into both states. `ready` shares
    // the triage floor, so the tile and the queue agree on what counts as met.
    let settings = db::load_settings(&state.local).await?;
    let unjudged =
        vocabulary::unjudged_counts(&state.knowledge, settings.triage_min_encounters).await?;

    // The sweep's own figure beside the standing one. From `triage_pending`
    // rather than recomputed, so the tile and the next tab's batch agree.
    let since = sweep_watermark(&state).await?;
    let ready_since =
        vocabulary::triage_pending(&state.knowledge, settings.triage_min_encounters, since)
            .await?
            .0;

    Ok(Json(json!({
        "total": total,
        "known_in_master": known,
        "known_words": words,
        "seen": unjudged.seen,
        "never_met": unjudged.never_met,
        "never_met_vocab": unjudged.never_met_vocab,
        "ready": unjudged.ready,
        "ready_since": ready_since,
        "swept_through": since,
        "ready_min_encounters": settings.triage_min_encounters,
        "by_status": by_status,
    })))
}

/// `/api/vocab/history` — the vocabulary count as a daily curve.
///
/// Counts **words**, not ledger rows, so the last point equals `known_words` on
/// the summary tile rather than the larger spelling count. A word's day is the
/// earliest day any of its spellings was called known: learning 辛い and then
/// calling つらい known months later is one word learnt once.
///
/// The curve only reaches back as far as `vocabulary_events` does. Everything
/// asserted before the log existed lands on the day the log's first entry
/// carries, which is the seeding — the first days are a bulk import, not a
/// week of reading.
pub async fn vocab_history(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let settings = db::load_settings(&state.local).await?;
    let tz = tz_offset_secs();

    let forms = lexeme::resolved_forms(&state.knowledge).await?;
    let mut earliest: HashMap<lexeme::Lexeme, f64> = HashMap::new();
    for (term, ts) in vocabulary::first_known_at(&state.knowledge).await? {
        let Some(word) = forms.get(&term) else {
            continue;
        };
        let slot = earliest.entry(word.clone()).or_insert(ts);
        *slot = slot.min(ts);
    }

    let dates: Vec<_> = earliest
        .values()
        .map(|ts| stats::date_key(*ts, settings.day_rollover_hour, tz))
        .collect();
    let today = stats::date_key(now_ts(), settings.day_rollover_hour, tz);
    let days = stats::growth_days(&dates, today);

    Ok(Json(json!({
        "days": days,
        "words": days.last().map(|d| d.cumulative).unwrap_or(0),
    })))
}

#[derive(Deserialize)]
pub struct QueueParams {
    /// Overrides the `triage_min_encounters` setting for one request, so the UI
    /// can preview what a threshold change does before saving it.
    min_encounters: Option<i64>,
    /// Scope the batch to what has been read since the last sweep (the
    /// default); `scoped=0` asks for the whole backlog. A string because
    /// `serde`'s `bool` accepts only `true`/`false` and would 400 on `0`.
    scoped: Option<String>,
    /// `frequency` sorts the same batch by frequency rank instead of by encounter
    /// count. Anything else is the encounter order.
    order: Option<String>,
}

/// When the last sweep was submitted, as an epoch timestamp in
/// `kotodex.db`'s settings.
///
/// Internal bookkeeping, so it sits outside `SETTING_KEYS` and the settings API
/// refuses to write it. A timestamp rather than a `lines.id` because it is
/// compared against `vocabulary.last_seen`. Absent means "never swept", which
/// reads as the whole backlog — a first sweep must not be empty.
const SWEEP_WATERMARK_KEY: &str = "sweep_through_ts";

async fn sweep_watermark(state: &AppState) -> Result<Option<f64>, AppError> {
    Ok(db::get_setting_raw(&state.local, SWEEP_WATERMARK_KEY)
        .await?
        .and_then(|v| v.parse().ok()))
}

/// The triage queue: untriaged vocabulary to judge, most-encountered first, or
/// commonest first with `order=frequency`. The ordering is a view of
/// one batch: it changes which rows the page limit reaches, nothing about what
/// the queue offers or what a submit writes.
///
/// `preselect` is computed here, not in the client — it decides what gets
/// written, so it has to be testable without a browser.
///
/// By default the batch is scoped to terms read since the last submit, so a
/// fortnight's reading produces a short list rather than the standing backlog.
/// The scoping is a filter and nothing more: it judges nothing, retires
/// nothing, and `scoped=0` still reaches every ready row.
pub async fn vocab_queue(
    State(state): State<AppState>,
    Query(params): Query<QueueParams>,
) -> Result<Json<Value>, AppError> {
    let settings = db::load_settings(&state.local).await?;
    let min = params
        .min_encounters
        .unwrap_or(settings.triage_min_encounters)
        .max(1);
    let since = match params.scoped.as_deref() {
        Some("0") | Some("false") => None,
        _ => sweep_watermark(&state).await?,
    };

    // Asking for frequency order without a frequency dictionary falls back to
    // encounters rather than failing the request.
    let order = match reader_frequency(&state).await? {
        Some(freq_id) if params.order.as_deref() == Some("frequency") => {
            vocabulary::QueueOrder::Frequency { freq_id }
        }
        _ => vocabulary::QueueOrder::Encounters,
    };
    let by_frequency = matches!(order, vocabulary::QueueOrder::Frequency { .. });

    let rows = vocabulary::triage_queue(&state.knowledge, min, since, order, QUEUE_LIMIT).await?;
    let (pending, pending_preselected) =
        vocabulary::triage_pending(&state.knowledge, min, since).await?;

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
                "freq_rank": r.freq_rank,
                "preselect": vocabulary::preselects_known(r, min),
            })
        })
        .collect();

    Ok(Json(json!({
        "min_encounters": min,
        "since": since,
        "scoped": since.is_some(),
        "order": if by_frequency { "frequency" } else { "encounters" },
        "pending": pending,
        "pending_preselected": pending_preselected,
        "terms": terms,
    })))
}

#[derive(Deserialize)]
pub struct SurfacesParams {
    headword: String,
    #[serde(default)]
    reading: String,
}

/// How one term was actually written, with a line to show for each spelling.
///
/// The ledger keys on Sudachi's normalized form, so a queue row reading 窺う may
/// never have appeared in kanji at all. Triage needs both halves of that: which
/// spellings, and a sentence per spelling — the only thing that separates a real
/// word from a tokenizer artefact without leaving the page.
pub async fn vocab_surfaces(
    State(state): State<AppState>,
    Query(params): Query<SurfacesParams>,
) -> Result<Json<Value>, AppError> {
    let term = Term::new(&params.headword, &params.reading);
    let rows = term_surfaces::for_term(&state.knowledge, &term).await?;

    let ids: Vec<i64> = rows.iter().filter_map(|r| r.line_id).collect();
    let texts = db::fetch_line_texts_by_id(&state.knowledge, &ids).await?;

    let surfaces: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "surface": r.surface,
                "count": r.count,
                "line": r.line_id.and_then(|id| texts.get(&id)),
            })
        })
        .collect();

    Ok(Json(json!({
        "headword": term.headword,
        "reading": term.display_reading(),
        "surfaces": surfaces,
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
    /// Move the sweep watermark to now, so the next batch is what the reading
    /// turns up after this one. Sent by the sweep and nothing else: a one-off
    /// judgement must not retire a batch nobody looked at.
    #[serde(default)]
    advance_sweep: bool,
}

/// Write a batch of judgements — the triage submit.
///
/// Statuses are parsed strictly rather than through `Status::parse`, whose
/// fallback to `new` would silently un-judge a row with a typo in it.
///
/// The sweep watermark advances **after** the write and only on the sweep's own
/// request — on submit, not on load, or an interrupted sweep loses its batch.
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

    let now = now_ts();
    let written = vocabulary::set_status_each(&state.knowledge, &judgements, now, "triage").await?;
    if req.advance_sweep {
        db::save_setting(&state.local, SWEEP_WATERMARK_KEY, &now.to_string()).await?;
    }
    let swept_through = req.advance_sweep.then_some(now);
    Ok(Json(
        json!({ "written": written, "swept_through": swept_through }),
    ))
}

/// Re-home every judgement the rebuild stranded.
///
/// A stranded row is one the reader judged that the ingest does not produce: a
/// normalization that folds いっぱい onto 一杯 leaves the old key behind. The
/// tokenizer says what each old key is called now, and the judgement moves onto
/// that row.
///
/// A row the tokenizer cannot resolve to a single token is left alone: a
/// stranded judgement is harmless, a misplaced one is not.
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
/// It is a bulk write over rows the queue never shows, so the reader would
/// otherwise be approving a predicate blind. Same `WHERE`, commonest first, and
/// paged rather than truncated — the tail is the part in question.
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

/// How many rows the browse list shows at a time.
const BROWSE_PAGE: i64 = 100;

#[derive(Deserialize)]
pub struct BrowseParams {
    pub status: Option<String>,
    /// `status_source`: `triage` is what was judged by hand, `seed` what an
    /// import claimed in bulk.
    pub source: Option<String>,
    /// Rarest first by default — the end of a bulk import where a threshold
    /// rule claims words that were never really met.
    pub common_first: Option<bool>,
    /// One row per ledger *form* rather than per word. Off by default: 元々 and
    /// 元元 are one word, and listing both is listing the database.
    pub all_forms: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Read the ledger rather than judge it: a page of rows, filtered by status and
/// by which pass wrote them, ordered by how common the corpus says they are.
///
/// The triage passes answer "what should I look at next". This one answers
/// "what is in here", which is the question a bulk import raises and nothing
/// else could show.
pub async fn vocab_browse(
    State(state): State<AppState>,
    Query(params): Query<BrowseParams>,
) -> Result<Json<Value>, AppError> {
    let limit = params.limit.unwrap_or(BROWSE_PAGE).clamp(1, 500);
    let offset = params.offset.unwrap_or(0).max(0);
    let blank_is_none = |s: &Option<String>| s.clone().filter(|v| !v.is_empty() && v != "any");
    let (total, rows) = vocabulary::browse(
        &state.knowledge,
        blank_is_none(&params.status).as_deref(),
        blank_is_none(&params.source).as_deref(),
        !params.common_first.unwrap_or(false),
        !params.all_forms.unwrap_or(false),
        limit,
        offset,
    )
    .await?;
    Ok(Json(json!({
        "total": total,
        "offset": offset,
        "limit": limit,
        "terms": rows
            .iter()
            .map(|r| json!({
                "headword": r.term.headword,
                "reading": r.term.display_reading(),
                "status": r.status,
                "source": r.source,
                "encounter_count": r.encounter_count,
                "lookup_count": r.lookup_count,
                "mined": r.mined,
                "rank": r.rank,
                "forms": r.forms,
            }))
            .collect::<Vec<_>>(),
    })))
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

/// Import the Anki review pile as `known`.
///
/// Reader-triggered only — never folded into `anki_refresh`'s recurring
/// snapshot, which must never write `status`. `-is:new -is:learn` is the gate:
/// a card still in Anki's new/learning queues is a word not yet had.
///
/// Anki carries no reading beside the vocab field, so each term is resolved
/// against the master dictionary: no match stores an empty reading, and a
/// homograph is skipped and counted rather than guessed at.
///
/// The card's spelling is normalized first, because a card is spelt the way the
/// text spelt it and the ledger keys on Sudachi's normalized form. Without it 検死
/// imports as its own row while every reading of it lands on 検屍, so the word is
/// known and unjudged at once. Only the spelling is normalized: the reading still
/// comes from the master dictionary, so a homograph is skipped rather than
/// resolved by whatever reading Sudachi picks for a word standing on its own.
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

    let spellings = crate::ingest::normalized_spellings(
        &state,
        notes.iter().map(|n| n.vocab.clone()).collect(),
    )
    .await?;

    let mut judgements = Vec::with_capacity(notes.len());
    let mut ambiguous_skipped = 0i64;
    for (note, spelling) in notes.iter().zip(&spellings) {
        let mut headword = spelling;
        let mut readings = dictionaries::master_readings(state.knowledge.pool(), headword).await?;
        // Normalizing onto a spelling the master does not list would trade a
        // reading for nothing — ボウガン becomes ボーガン, which Sankoku has no
        // entry for. Keep the card's own spelling in that case.
        if readings.is_empty() && headword != &note.vocab {
            headword = &note.vocab;
            readings = dictionaries::master_readings(state.knowledge.pool(), headword).await?;
        }
        match readings.as_slice() {
            [] => judgements.push((Term::new(headword.clone(), ""), Status::Known)),
            [reading] => judgements.push((Term::new(headword.clone(), reading), Status::Known)),
            _ => ambiguous_skipped += 1,
        }
    }

    let imported =
        vocabulary::set_status_each(&state.knowledge, &judgements, now_ts(), "anki").await?;
    Ok(Json(json!({
        "imported": imported,
        "ambiguous_skipped": ambiguous_skipped,
    })))
}

/// Repair the empty-reading rows the Anki import creates for kanji headwords.
///
/// Reader-triggered rather than automatic, because it merges and deletes rows.
/// Idempotent.
pub async fn vocab_repair_empty_readings(
    State(state): State<AppState>,
) -> Result<Json<Value>, AppError> {
    let r = vocabulary::repair_empty_readings(&state.knowledge).await?;
    vocabulary::refresh_dictionary_flags(&state.knowledge).await?;
    info!(
        rekeyed = r.rekeyed,
        merged = r.merged,
        unresolved = r.unresolved,
        "reading repair"
    );
    Ok(Json(json!({
        "rekeyed": r.rekeyed,
        "merged": r.merged,
        "unresolved": r.unresolved,
    })))
}

/// The frequency list triage ranks against, by title since it carries no role.
/// `None` where no dictionary holds the frequency role: the sweep then orders
/// by encounters and a per-work list carries no rank, which is a smaller
/// product rather than an error.
///
/// The sweep's ordering and the reader's underline must rank off the *same*
/// list — they are one claim about which words are common, made in two places —
/// and it is not the tokenizer's BCCWJ. `dictionaries::reader_frequency` says
/// why.
pub(crate) async fn reader_frequency(state: &AppState) -> Result<Option<i64>, AppError> {
    Ok(dictionaries::reader_frequency(state.knowledge.pool())
        .await?
        .map(|d| d.id))
}

/// Rebuild the ledger's counts from the whole reading history.
///
/// Zeroes the aggregates, rewinds only the ledger's watermarks, and re-runs both
/// ingests; `word_days` is untouched because its own watermarks stay put.
/// Assertions survive — `status` is not a count.
///
/// This is the repair path for a re-tokenization: a Sudachi upgrade, or a change
/// to what counts as a content word.
pub async fn vocab_rebuild(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    crate::ingest::reset_vocabulary(&state).await?;
    let lines = crate::ingest::ingest_new_lines(&state).await?;
    let sessions = crate::ingest::ingest_new_sessions(&state).await?;
    let mined = crate::ingest::sync_vocabulary(&state).await?;
    // A re-tokenization moves words between keys. Carry the judgements before
    // pruning, or the next step deletes them.
    let carried = carry_stranded_judgements(&state).await?;
    // Anything the re-ingest did not touch is no longer in the reading. Judged
    // and mined rows are spared.
    let pruned = vocabulary::prune_untouched(&state.knowledge).await?;

    Ok(Json(json!({
        "lines": lines,
        "sessions": sessions,
        "mined_terms": mined,
        "carried": carried,
        "pruned": pruned,
    })))
}
