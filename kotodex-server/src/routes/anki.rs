//! `/api/anki/*` — refreshing the deck snapshot, what it says about
//! re-encounters, and the card report.

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use serde_json::{Value, json};

use crate::app::AppState;
use crate::clock::{now_ts, tz_offset_secs};
use crate::db;
use crate::error::AppError;
use crate::stats::{self, card_evidence};

/// Probe for AnkiConnect (dashboard client first, then the configured
/// fallback), snapshot the mined deck, then tokenize any new lines.
pub async fn anki_refresh(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Result<Json<Value>, AppError> {
    let mut last_err = AppError::Upstream(format!(
        "no AnkiConnect reachable (tried dashboard client {} and {})",
        addr.ip(),
        state.anki_url
    ));
    let mut snapshot = None;
    for url in crate::services::anki::candidate_urls(Some(addr.ip()), &state.anki_url) {
        match crate::services::anki::fetch_deck_vocab(
            &state.http,
            &url,
            &state.anki_deck,
            &state.anki_vocab_field,
        )
        .await
        {
            Ok(notes) => {
                snapshot = Some((url, notes));
                break;
            }
            Err(e) => last_err = e,
        }
    }
    let Some((source, notes)) = snapshot else {
        return Err(last_err);
    };

    // Normalized before the snapshot lands, so every join against a tokenizer
    // lemma has a key to use rather than the card's own spelling.
    let spellings = crate::ingest::normalized_spellings(
        &state,
        notes.iter().map(|n| n.vocab.clone()).collect(),
    )
    .await?;
    let notes: Vec<db::AnkiNote> = notes
        .into_iter()
        .zip(spellings)
        .map(|(n, headword)| db::AnkiNote { headword, ..n })
        .collect();

    db::replace_anki_notes(&state.knowledge, &notes).await?;
    db::save_setting(&state.local, "anki_snapshot_ts", &now_ts().to_string()).await?;
    db::save_setting(&state.local, "anki_source", &source).await?;
    let ingest = crate::ingest::ingest_new_lines(&state).await?;
    let session_ingest = crate::ingest::ingest_new_sessions(&state).await?;
    // After the ingest, never before: the syncs mark rows the ingest may have
    // only just created.
    let mined = crate::ingest::sync_vocabulary(&state).await?;

    Ok(Json(json!({
        "notes": notes.len(),
        "source": source,
        "ingest": ingest,
        "session_ingest": session_ingest,
        "mined_terms": mined,
    })))
}

/// `GET /api/anki/up` — is AnkiConnect answering where cards are added?
///
/// `state.anki_url` alone, not [`services::anki::candidate_urls`]: this reports
/// on the path a mine actually takes, and `services::card` posts there.
///
/// Its own endpoint rather than a field on the reader's status event, because
/// the probe waits out its timeout whenever nothing answers and that event
/// shares a loop with the line feed.
///
/// `mining_used` says whether this install mines at all. Anki is optional, so a
/// surface that would warn about it asks this first: a reader who has never
/// mined a card is not missing anything.
pub async fn anki_up(State(state): State<AppState>) -> Json<Value> {
    let up = crate::services::anki::reachable(&state.http, &state.anki_url).await;
    let mining_used = db::any_anki_note(&state.knowledge).await.unwrap_or(false);
    Json(json!({ "up": up, "mining_used": mining_used }))
}

/// `GET /api/anki/cards` — every mined card against what the reading knows.
///
/// Read-only, and deliberately so: it reports what a sweep *would* act on. The
/// deck's scheduling state has to come from Anki live, since `anki_notes`
/// mirrors only which words are carded, not what Anki has learnt about them.
///
/// The join is on [`db::AnkiNote::key`], the resolved ledger key — never the
/// card's own spelling. A card saying 検死 against a ledger row of 検屍 would
/// otherwise read as never met and land in the retire pile.
pub async fn anki_cards(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Result<Json<Value>, AppError> {
    let Some(url) =
        crate::services::anki::pick_url(&state.http, Some(addr.ip()), &state.anki_url).await
    else {
        return Ok(Json(json!({ "available": false })));
    };

    let cards =
        crate::services::anki::fetch_deck_cards(&state.http, &url, &state.anki_deck).await?;
    // The real last-review time per card. `mod` cannot stand in for it — see
    // `CardStat::modified`.
    let reviews =
        crate::services::anki::fetch_deck_reviews(&state.http, &url, &state.anki_deck).await?;

    let settings = db::load_settings(&state.local).await?;
    let tz = tz_offset_secs();
    let rollover = settings.day_rollover_hour;
    let day_of = |ts: f64| stats::date_key(ts, rollover, tz).to_string();

    let notes: BTreeMap<i64, db::AnkiNote> = db::fetch_anki_notes(&state.knowledge)
        .await?
        .into_iter()
        .map(|n| (n.note_id, n))
        .collect();

    let mut word_days: HashMap<String, Vec<(String, i64)>> = HashMap::new();
    for hit in db::fetch_word_days(&state.knowledge).await? {
        word_days
            .entry(hit.lemma)
            .or_default()
            .push((hit.date, hit.count));
    }
    let mut lookups: HashMap<String, Vec<f64>> = HashMap::new();
    for (key, ts) in db::fetch_lookup_keys(&state.knowledge).await? {
        lookups.entry(key).or_default().push(ts);
    }

    // A card whose note is missing from the mirror has no ledger key to join
    // on. Counted rather than guessed at: the answer is a refresh, not a
    // fallback to the card's own spelling.
    let mut unmirrored = 0;
    // A card Anki has no review row for. Its cutoff falls back to `mod`, so the
    // number is worth showing rather than hiding inside the buckets.
    let mut unlogged = 0;
    let inputs: Vec<card_evidence::CardInput> = cards
        .iter()
        .filter_map(|c| {
            let Some(note) = notes.get(&c.note_id) else {
                unmirrored += 1;
                return None;
            };
            // A suspended card is already out of the rotation; nothing here
            // would have anything to say about it.
            if c.suspended {
                return None;
            }
            let created_ts = c.note_id as f64 / 1000.0;
            let last_review_ts = reviews.get(&c.card_id).copied().unwrap_or_else(|| {
                unlogged += 1;
                c.modified
            });
            Some(card_evidence::CardInput {
                note_id: c.note_id,
                vocab: note.vocab.clone(),
                key: note.key().to_string(),
                interval: c.interval,
                lapses: c.lapses,
                is_review: c.is_review(),
                last_review_ts,
                created_ts,
                last_review_day: day_of(last_review_ts),
                created_day: day_of(created_ts),
            })
        })
        .collect();

    let evidence = card_evidence::evaluate(&card_evidence::Inputs {
        cards: &inputs,
        word_days: &word_days,
        lookups: &lookups,
        now: now_ts(),
    });

    // Each bucket is its own list, most evidence first — the top of a list is
    // where a sweep would start and where a wrong threshold shows itself.
    let mut buckets: BTreeMap<String, Vec<&card_evidence::CardEvidence>> = BTreeMap::new();
    for e in &evidence {
        let Some(bucket) = e.bucket else { continue };
        let name = serde_json::to_value(bucket)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        buckets.entry(name).or_default().push(e);
    }
    for (name, rows) in buckets.iter_mut() {
        match name.as_str() {
            "bring_forward" => rows.sort_by(|a, b| b.interval.cmp(&a.interval)),
            "never_met" => rows.sort_by_key(|r| r.note_id),
            _ => rows.sort_by(|a, b| b.encounter_days.cmp(&a.encounter_days)),
        }
    }
    let counts: BTreeMap<&String, usize> = buckets.iter().map(|(k, v)| (k, v.len())).collect();
    let listed: BTreeMap<&String, Vec<&card_evidence::CardEvidence>> = buckets
        .iter()
        .map(|(k, v)| (k, v.iter().take(BUCKET_SAMPLE).copied().collect()))
        .collect();

    Ok(Json(json!({
        "available": true,
        "source": url,
        "deck": state.anki_deck,
        "cards": inputs.len(),
        "reviewing": inputs.iter().filter(|c| c.is_review).count(),
        "unmirrored": unmirrored,
        "unlogged": unlogged,
        "listed_per_bucket": BUCKET_SAMPLE,
        "counts": counts,
        "buckets": listed,
        "thresholds": {
            "mature_days": card_evidence::MATURE_DAYS,
            "defer_days": card_evidence::DEFER_DAYS,
            "retire_days": card_evidence::RETIRE_DAYS,
            "retire_interval": card_evidence::RETIRE_INTERVAL,
            "never_met_age_days": card_evidence::NEVER_MET_AGE_DAYS,
        },
    })))
}

/// How many rows of each bucket travel with the report. The counts are whole;
/// the lists are for looking at, and a bucket of 900 is judged from its head.
const BUCKET_SAMPLE: usize = 200;

/// Re-encounter statistics: how often mined words reappear in the line stream.
pub async fn anki_summary(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let Some(snapshot_ts) = db::get_setting_raw(&state.local, "anki_snapshot_ts")
        .await?
        .and_then(|v| v.parse::<f64>().ok())
    else {
        return Ok(Json(json!({ "available": false })));
    };

    let settings = db::load_settings(&state.local).await?;
    let tz = tz_offset_secs();
    let rollover = settings.day_rollover_hour;
    let today = stats::date_key(now_ts(), rollover, tz);
    let week_start = (today - chrono::Duration::days(6)).to_string();

    // Earliest note per vocab (dupes possible when a word was re-mined).
    let mut mined: BTreeMap<String, (i64, String)> = BTreeMap::new();
    for n in db::fetch_anki_notes(&state.knowledge).await? {
        let date = stats::date_key(n.note_id as f64 / 1000.0, rollover, tz).to_string();
        mined
            .entry(n.key().to_string())
            .or_insert((n.note_id, date));
    }

    // Encounters per mined lemma, split into after-mined-day and last-7-days.
    let mut after: BTreeMap<&str, i64> = BTreeMap::new();
    let mut week: BTreeMap<&str, i64> = BTreeMap::new();
    let hits = db::fetch_mined_word_days(&state.knowledge).await?;
    for h in &hits {
        let Some((_, mined_date)) = mined.get(&h.lemma) else {
            continue;
        };
        if h.date > *mined_date {
            *after.entry(h.lemma.as_str()).or_default() += h.count;
        }
        if h.date >= week_start {
            *week.entry(h.lemma.as_str()).or_default() += h.count;
        }
    }

    let reencountered = after.len() as i64;
    let week_total: i64 = week.values().sum();
    let mut top_week: Vec<_> = week.iter().map(|(w, c)| (*w, *c)).collect();
    top_week.sort_by(|a, b| b.1.cmp(&a.1));
    let top_week: Vec<Value> = top_week
        .iter()
        .take(10)
        .map(|(w, c)| json!({ "word": w, "count": c }))
        .collect();

    // Never re-encountered since mined, oldest cards first.
    let mut never: Vec<_> = mined
        .iter()
        .filter(|(vocab, _)| !after.contains_key(vocab.as_str()))
        .map(|(vocab, (note_id, _))| (*note_id, vocab.clone()))
        .collect();
    never.sort();

    Ok(Json(json!({
        "available": true,
        "snapshot_ts": snapshot_ts,
        "source": db::get_setting_raw(&state.local, "anki_source").await?,
        "mined": mined.len(),
        "reencountered": reencountered,
        "week_encounters": week_total,
        "top_week": top_week,
        "never_count": never.len(),
        "never_sample": never.iter().take(10).map(|(_, w)| w).collect::<Vec<_>>(),
    })))
}
