//! `/api/anki/*` — refreshing the deck snapshot, and what it says about
//! re-encounters.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use serde_json::{Value, json};

use crate::app::AppState;
use crate::clock::{now_ts, tz_offset_secs};
use crate::db;
use crate::error::AppError;
use crate::stats;

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

    db::replace_anki_notes(&state.pool, &notes).await?;
    db::save_setting(&state.pool, "anki_snapshot_ts", &now_ts().to_string()).await?;
    db::save_setting(&state.pool, "anki_source", &source).await?;
    let ingest = crate::ingest::ingest_new_lines(&state).await?;

    Ok(Json(
        json!({ "notes": notes.len(), "source": source, "ingest": ingest }),
    ))
}

/// Re-encounter statistics: how often mined words reappear in the line stream.
pub async fn anki_summary(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let Some(snapshot_ts) = db::get_setting_raw(&state.pool, "anki_snapshot_ts")
        .await?
        .and_then(|v| v.parse::<f64>().ok())
    else {
        return Ok(Json(json!({ "available": false })));
    };

    let settings = db::load_settings(&state.pool).await?;
    let tz = tz_offset_secs();
    let rollover = settings.day_rollover_hour;
    let today = stats::date_key(now_ts(), rollover, tz);
    let week_start = (today - chrono::Duration::days(6)).to_string();

    // Earliest note per vocab (dupes possible when a word was re-mined).
    let mut mined: BTreeMap<String, (i64, String)> = BTreeMap::new();
    for n in db::fetch_anki_notes(&state.pool).await? {
        let date = stats::date_key(n.note_id as f64 / 1000.0, rollover, tz).to_string();
        mined.entry(n.vocab).or_insert((n.note_id, date));
    }

    // Encounters per mined lemma, split into after-mined-day and last-7-days.
    let mut after: BTreeMap<&str, i64> = BTreeMap::new();
    let mut week: BTreeMap<&str, i64> = BTreeMap::new();
    let hits = db::fetch_mined_word_days(&state.pool).await?;
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
        "source": db::get_setting_raw(&state.pool, "anki_source").await?,
        "mined": mined.len(),
        "reencountered": reencountered,
        "week_encounters": week_total,
        "top_week": top_week,
        "never_count": never.len(),
        "never_sample": never.iter().take(10).map(|(_, w)| w).collect::<Vec<_>>(),
    })))
}
