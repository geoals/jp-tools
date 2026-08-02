//! `/api/vocab/*` — the knowledge ledger's status endpoints. The ledger itself
//! is `jp_core::knowledge::vocabulary`.
//!
//! The rule these handlers exist to keep: **`status` is only ever written from a
//! request the reader made.** No sync touches it, so the ledger cannot demote a
//! word behind their back and an encounter count cannot promote one.

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
use crate::clock::now_ts;
use crate::db;
use crate::error::AppError;

/// Rows per queue page — one sweep of attention.
const QUEUE_LIMIT: i64 = 200;

/// Rows per page of the non-vocabulary tail. The whole set is reachable by
/// paging.
const NON_WORD_PAGE: i64 = 100;

/// Rows per page of frequency-triage candidates. Large because homographs are
/// filtered out before paging, so every row is one the button can act on.
const FREQUENCY_PAGE: i64 = 500;

/// What the ledger holds, by status.
///
/// `in_master` is the vocabulary scale: a term counts toward "I know N words"
/// only if the master dictionary lists it. Jitendex's 400k entries are a phrase
/// index and would make the number meaningless.
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

#[derive(Deserialize)]
pub struct QueueParams {
    /// Overrides the `triage_min_encounters` setting for one request, so the UI
    /// can preview what a threshold change does before saving it.
    min_encounters: Option<i64>,
    /// Scope the batch to what has been read since the last sweep (the
    /// default); `scoped=0` asks for the whole backlog. A string because
    /// `serde`'s `bool` accepts only `true`/`false` and would 400 on `0`.
    scoped: Option<String>,
    /// `frequency` sorts the same batch by BCCWJ rank instead of by encounter
    /// count. Anything else is the encounter order.
    order: Option<String>,
}

/// When the last sweep was submitted, as an epoch timestamp in
/// `read-stats.db`'s settings.
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
/// commonest in BCCWJ first with `order=frequency`. The ordering is a view of
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

    // The ordering picks which end of the same batch is on screen, so the
    // pending counts below hold for both.
    let by_frequency = params.order.as_deref() == Some("frequency");
    let order = if by_frequency {
        let bccwj = dictionaries::by_title(state.knowledge.pool(), "BCCWJ")
            .await?
            .ok_or_else(|| AppError::Upstream("BCCWJ frequency dictionary not loaded".into()))?;
        vocabulary::QueueOrder::Frequency { bccwj_id: bccwj.id }
    } else {
        vocabulary::QueueOrder::Encounters
    };

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
/// A stranded row is one the reader judged that the ingest no longer produces —
/// いっぱい became 一杯 when headwords were normalized. The tokenizer says what
/// each old key is called now, and the judgement moves onto that row.
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
    /// `status_source`: `seed` is the jiten import, `triage` what was judged by
    /// hand.
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
/// "what is in here", which is the question a 5,000-word import raises and
/// nothing else could show.
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

    let imported =
        vocabulary::set_status_each(&state.knowledge, &judgements, now_ts(), "anki").await?;
    Ok(Json(json!({
        "imported": imported,
        "ambiguous_skipped": ambiguous_skipped,
    })))
}

/// jiten.moe's JSON export: cards keyed by JMdict entry id.
///
/// Only `w` is read. The status field `s` is ignored and every card imports as
/// `known` — jiten's maturity grades are its scheduler's business.
#[derive(Deserialize)]
pub struct JitenExport {
    cards: Vec<JitenCard>,
}

#[derive(Deserialize)]
pub struct JitenCard {
    /// JMdict `ent_seq`. The same id `dictionary_entries.sequence` stores.
    w: i64,
}

#[derive(Deserialize)]
pub struct JitenImportParams {
    /// Refuse anything the frequency list ranks at or past this. Absent means
    /// no gate, which is the old behaviour.
    pub max_rank: Option<i64>,
    /// Which loaded frequency list to rank against. `Jiten` by default — it is
    /// built from the same kind of material the reading is, where BCCWJ is
    /// newspapers and books.
    pub rank_list: Option<String>,
}

/// See [`JitenImportParams::rank_list`].
fn rank_source(params: &JitenImportParams) -> String {
    params.rank_list.clone().unwrap_or_else(|| "Jiten".into())
}

/// Seed the ledger from a jiten.moe export.
///
/// The only source that names *words* rather than spellings: it carries JMdict
/// entry ids, so 辛い/つらい is marked and 辛い/からい is not, with nothing
/// guessed. Hence no ambiguous-skipped count — there is no ambiguity.
///
/// An id fans out to every spelling the **master** dictionary lists. That is
/// safe because counting collapses back — `lexeme` reports こちら and こっち as
/// one word — and it stops triage asking about each spelling separately.
///
/// **A word the reader has looked up is never imported**, whatever the list
/// says — see [`vocabulary::looked_up_headwords`]. The list says a word was met
/// often enough to have been learnt; a lookup says it was not, and only one of
/// the two is the reader's own evidence.
///
/// **`max_rank` gates on rarity**, off unless asked for. See the retain below.
///
/// Order does not matter and re-running is free: `status` is set, never
/// cleared, and counts are left alone. Undone by
/// [`vocab_seed_undo`] rather than by hand.
pub async fn vocab_jiten_import(
    State(state): State<AppState>,
    Query(params): Query<JitenImportParams>,
    Json(export): Json<JitenExport>,
) -> Result<Json<Value>, AppError> {
    let forms_by_seq = dictionaries::master_forms_by_sequence(state.knowledge.pool()).await?;
    let labels = dictionaries::entry_labels_by_sequence(state.knowledge.pool()).await?;
    let looked_up = vocabulary::looked_up_headwords(&state.knowledge).await?;
    let too_rare_gate = match params.max_rank {
        Some(max) if max > 0 => {
            let corpus = dictionaries::by_title(state.knowledge.pool(), &rank_source(&params))
                .await?
                .ok_or_else(|| {
                    AppError::Upstream(format!("{} frequency list not loaded", rank_source(&params)))
                })?;
            Some((corpus.id, max))
        }
        _ => None,
    };

    let ids: std::collections::HashSet<i64> = export.cards.iter().map(|c| c.w).collect();
    let mut terms: std::collections::HashSet<Term> = std::collections::HashSet::new();
    let mut unresolved: Vec<Value> = Vec::new();
    for id in &ids {
        match forms_by_seq.get(id) {
            // No master spelling: a name, or one of JMdict's phrase entries.
            // Not vocabulary, so it imports as nothing rather than as a row
            // the triage queue would then have to reject.
            None => {
                let (term, reading) = labels
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| (id.to_string(), String::new()));
                unresolved.push(json!({ "term": term, "reading": reading }));
            }
            Some(forms) => {
                for (headword, reading) in forms {
                    terms.insert(Term::new(headword.clone(), reading));
                }
            }
        }
    }
    unresolved.sort_by_key(|v| v["term"].as_str().unwrap_or_default().to_string());

    // **Rarer than the gate is not claimed.** A "met N times" threshold is
    // sound for common words and weak for rare ones, where N sightings are one
    // book's subject matter rather than the language. Ranked against a corpus
    // of the same kind of material the reading is — jiten's media list, not
    // BCCWJ, which is newspapers and under-ranks 結界 and クラスメイト by a
    // factor of three.
    //
    // A term the list does not rank is *kept*. Absence is not rarity: BCCWJ has
    // no rank for お辞儀 at all, and jiten leaves 38 of 4,316 unranked.
    let mut too_rare: Vec<Value> = Vec::new();
    if let Some((corpus, max)) = too_rare_gate {
        let headwords: Vec<String> = terms.iter().map(|t| t.headword.clone()).collect();
        let ranks =
            dictionaries::frequency_ranks(state.knowledge.pool(), corpus, &headwords).await?;
        terms.retain(|t| {
            // The pair first — "how common is this spelling read this way" —
            // then the headword under any reading, for a kana term the ledger
            // stores with no reading of its own.
            let said = jp_core::text::kana::to_hiragana(&t.reading);
            let rank = ranks
                .get(&(t.headword.clone(), said))
                .or_else(|| {
                    ranks.get(&(
                        t.headword.clone(),
                        jp_core::text::kana::to_hiragana(&t.headword),
                    ))
                })
                .copied();
            match rank {
                Some(r) if r >= max => {
                    too_rare.push(json!({ "term": t.headword, "reading": t.reading, "rank": r }));
                    false
                }
                _ => true,
            }
        });
        too_rare.sort_by_key(|v| std::cmp::Reverse(v["rank"].as_i64().unwrap_or(0)));
    }

    // Reading it in a list is not knowing it. The lookup log is the reader's
    // own record of the opposite, so it overrules the list: a word stopped for
    // goes to triage as `new` rather than being claimed here.
    let mut vetoed: Vec<Value> = Vec::new();
    terms.retain(|t| {
        if looked_up.contains(&t.headword) {
            vetoed.push(json!({ "term": t.headword, "reading": t.reading }));
            false
        } else {
            true
        }
    });
    vetoed.sort_by_key(|v| v["term"].as_str().unwrap_or_default().to_string());

    let judgements: Vec<(Term, Status)> = terms.into_iter().map(|t| (t, Status::Known)).collect();
    let marked = vocabulary::seed_status_each(&state.knowledge, &judgements, now_ts()).await?;

    // Most of these rows did not exist a moment ago, and a fresh row's
    // dictionary flags are zero until something fills them. Without this the
    // vocabulary scale, which gates on `in_master`, ignores the whole import.
    vocabulary::refresh_dictionary_flags(&state.knowledge).await?;

    info!(
        cards = export.cards.len(),
        ids = ids.len(),
        resolved = ids.len() - unresolved.len(),
        unresolved = unresolved.len(),
        vetoed = vetoed.len(),
        too_rare = too_rare.len(),
        marked,
        "jiten import"
    );
    Ok(Json(json!({
        "cards": export.cards.len(),
        "ids": ids.len(),
        "resolved_entries": ids.len() - unresolved.len(),
        "unresolved_entries": unresolved.len(),
        "vetoed_by_lookup": vetoed.len(),
        "too_rare": too_rare.len(),
        "terms_marked": marked,
        // Named, not counted: a drop the reader cannot read is one they cannot
        // check, and this is where the names and the phrase entries go.
        "dropped": { "unresolved": unresolved, "vetoed": vetoed, "too_rare": too_rare },
    })))
}

/// Put the last jiten import back, because a bulk claim about 14,000 words is
/// one worth being able to withdraw.
///
/// Exact rather than reconstructed: [`seed_status_each`] writes only over rows
/// that were `new`, so `new` is what every seeded row was, and the whole batch
/// shares one `status_ts`. Anything judged by hand since carries
/// `status_source = 'triage'` and is not touched — the undo withdraws the
/// import's claims and none of the reader's.
///
/// `vocabulary_events` keeps both the import and this, so the history says what
/// was claimed and what was taken back.
///
/// [`seed_status_each`]: jp_core::knowledge::vocabulary::seed_status_each
pub async fn vocab_seed_undo(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let reverted = vocabulary::undo_last_seed(&state.knowledge).await?;
    info!(reverted, "withdrew the last seed import");
    Ok(Json(json!({ "reverted": reverted })))
}

/// How many encounters make a non-master term worth asking about when no card
/// exists for it. A mined term is already the reader's own claim; a read one has
/// only the tokenizer's word for it, and at 1 the queue fills with noise.
const PROMOTION_MIN_ENCOUNTERS: i64 = 5;

/// Rows per page of promotion candidates. Small: a judgement per row, not a
/// sweep.
const PROMOTION_PAGE: i64 = 200;

/// The escape hatch's queue: terms the master dictionary does not list but the
/// reader has mined or read repeatedly.
///
/// Sankoku carries no 冪等性 or 可用性, nor a stem of either, so no
/// decomposition rule reaches them; JMdict would admit them along with every
/// idiom and orthographic variant. So the reader decides one term at a time.
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
/// Never touches `status`: promoting 冪等性 asserts it is a word, not that it is
/// known.
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

/// How many BCCWJ terms at or under a rank threshold are unjudged master
/// vocabulary — the cheap count the threshold slider previews against.
pub async fn vocab_frequency_summary(
    State(state): State<AppState>,
    Query(params): Query<FrequencyParams>,
) -> Result<Json<Value>, AppError> {
    let settings = db::load_settings(&state.local).await?;
    let max_rank = params
        .max_rank
        .unwrap_or(settings.triage_max_freq_rank)
        .max(1);
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

/// A page of frequency-triage candidates — what `frequency-commit` would act
/// on, shown first, the same rule as `vocab_non_words`.
///
/// Homographs are excluded at the query rather than at display, so every row is
/// one the commit button can write. `ambiguous` is still reported, for the line
/// that says how many words are being left out and why.
pub async fn vocab_frequency_queue(
    State(state): State<AppState>,
    Query(params): Query<FrequencyParams>,
) -> Result<Json<Value>, AppError> {
    let settings = db::load_settings(&state.local).await?;
    let max_rank = params
        .max_rank
        .unwrap_or(settings.triage_max_freq_rank)
        .max(1);
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
/// The threshold persists into `triage_max_freq_rank`.
///
/// `frequency_queue` already excludes homographs and already-judged rows, so
/// nothing is left to resolve here — including a word the reader declined while
/// previewing, which `vocab_judge` has already given a status.
/// `ambiguous_skipped` comes from `frequency_pending` rather than being counted
/// again, since a second count could only drift from it.
pub async fn vocab_frequency_commit(
    State(state): State<AppState>,
    Json(req): Json<FrequencyCommitRequest>,
) -> Result<Json<Value>, AppError> {
    let max_rank = req.max_rank.max(1);
    let (bccwj, master) = frequency_dictionaries(&state).await?;

    let pending = vocabulary::frequency_pending(&state.knowledge, bccwj, master, max_rank).await?;
    // Unbounded (SQLite's own convention: LIMIT -1 means no limit), at this
    // scale a few thousand rows even at a generous threshold.
    let rows =
        vocabulary::frequency_queue(&state.knowledge, bccwj, master, max_rank, -1, 0).await?;
    let judgements: Vec<(Term, Status)> = rows
        .iter()
        .map(|r| (Term::new(r.term.clone(), &r.reading), Status::Known))
        .collect();

    let written =
        vocabulary::set_status_each(&state.knowledge, &judgements, now_ts(), "frequency").await?;
    db::save_setting(&state.local, "triage_max_freq_rank", &max_rank.to_string()).await?;
    Ok(Json(
        json!({ "written": written, "ambiguous_skipped": pending.ambiguous }),
    ))
}

/// The two dictionary ids frequency triage joins against — BCCWJ (by title,
/// since it carries no role) and the master dictionary. Both load at startup,
/// so a missing one is a deployment problem, not a retryable request error.
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
