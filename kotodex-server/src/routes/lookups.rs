//! `/api/lookups/summary` — the mining funnel.
//!
//! How many lookups turn into cards, and which ones didn't work out.
//!
//! Three outcomes per distinct term, decided by comparing the card's creation
//! time (the note id, epoch ms) against the term's first lookup:
//!   - **mined** — a card was made at or after the lookup: the lookup stuck.
//!   - **known** — a card already existed: a word that was mined but didn't
//!     take, i.e. a leech worth reformulating.
//!   - **unmined** — looked up, never carded. Repeats here are mining
//!     candidates: the same word slowed you down more than once.
//!
//! Counts are over *distinct terms*, not lookup events, so a word looked up
//! five times before being mined counts once and can't inflate the rate.

use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::clock::now_ts;
use crate::db::{self, LookupTerm};
use crate::error::AppError;

/// How many terms each list carries back to the dashboard.
const LIST_CAP: usize = 12;

fn status_of(t: &LookupTerm) -> &'static str {
    match t.note_id {
        Some(id) if id as f64 / 1000.0 >= t.first_ts => "mined",
        Some(_) => "known",
        None => "unmined",
    }
}

pub async fn lookups_summary(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let terms = db::fetch_lookup_terms(&state.knowledge).await?;

    let (mut mined, mut known, mut unmined) = (0i64, 0i64, 0i64);
    let mut leeches: Vec<&LookupTerm> = Vec::new();
    // Lookup → card latency, a first read on what mining actually costs.
    let mut mine_lags: Vec<f64> = Vec::new();

    for t in &terms {
        match status_of(t) {
            "mined" => {
                mined += 1;
                // From the lookup that led to the card, not the first ever.
                if let Some(from) = t.mine_from_ts {
                    mine_lags.push(t.note_id.unwrap() as f64 / 1000.0 - from);
                }
            }
            "known" => {
                known += 1;
                leeches.push(t);
            }
            _ => unmined += 1,
        }
    }

    // Words looked up more than once, worst first — the ones costing repeat
    // time. Status rides along: an unmined repeat is a mining candidate, a
    // known repeat is a card that isn't working.
    let mut repeats: Vec<&LookupTerm> = terms.iter().filter(|t| t.times > 1).collect();

    // Worst first: most re-looked-up, then most recent.
    let by_weight = |a: &&LookupTerm, b: &&LookupTerm| {
        b.times.cmp(&a.times).then(b.last_ts.total_cmp(&a.last_ts))
    };
    leeches.sort_by(by_weight);
    repeats.sort_by(by_weight);

    mine_lags.sort_by(f64::total_cmp);
    let median_mine_secs = (!mine_lags.is_empty()).then(|| mine_lags[mine_lags.len() / 2]);

    let brief = |list: &[&LookupTerm]| -> Vec<Value> {
        list.iter()
            .take(LIST_CAP)
            .map(|t| {
                json!({
                    "term": t.term,
                    "times": t.times,
                    "last_ts": t.last_ts,
                    "status": status_of(t),
                    // Days since the card was made — a long-standing card still
                    // being looked up is the strongest leech signal.
                    "card_age_days": t.note_id.map(|id| {
                        ((now_ts() - id as f64 / 1000.0) / 86400.0).floor()
                    }),
                })
            })
            .collect()
    };

    Ok(Json(json!({
        "terms": terms.len(),
        "events": terms.iter().map(|t| t.times).sum::<i64>(),
        "mined": mined,
        "known": known,
        "unmined": unmined,
        "median_mine_secs": median_mine_secs,
        "repeat_terms": repeats.len(),
        // Lookups spent re-reading a word already looked up before.
        "repeat_events": repeats.iter().map(|t| t.times - 1).sum::<i64>(),
        "repeats": brief(&repeats),
        "leeches": brief(&leeches),
        "leech_count": leeches.len(),
    })))
}
