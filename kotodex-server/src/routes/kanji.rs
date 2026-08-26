//! `/api/kanji` — every kanji ever read, in one payload.
//!
//! The whole tab is one request on purpose. Its cards are several readings of
//! the same rows, and letting each fetch its own would let the grid and the
//! grade meters disagree about a kanji met while the page was open. It is
//! the same rule the dashboard's single poll follows, applied to a heavier
//! aggregate: a couple of thousand rows, small enough to slice on the client
//! and large enough that slicing it twice on the server would be waste.

use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::clock::tz_offset_secs;
use crate::db;
use crate::error::AppError;
use crate::stats::{TermTimes, aggregate_kanji, kanji::SOLID_ENCOUNTERS};

pub async fn kanji(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let settings = db::load_settings(&state.local).await?;
    let mut lines = db::fetch_kanji_lines(&state.knowledge).await?;
    // Text pasted into a logged session counts as reading: its kanji were met,
    // so it feeds every figure on this tab. Articles aggregate under one work
    // rather than one per headline.
    for s in db::fetch_session_texts_after(&state.knowledge, 0).await? {
        lines.push(crate::stats::KanjiLine {
            ts: s.start_ts,
            text: s.content,
            work: crate::stats::work_key(&s.source, s.work.as_deref()),
        });
    }
    // `aggregate_kanji` walks in time order for first-encounter dates.
    lines.sort_by(|a, b| a.ts.total_cmp(&b.ts));
    let lookups: Vec<TermTimes> = db::fetch_lookup_terms(&state.knowledge)
        .await?
        .into_iter()
        .map(|t| TermTimes {
            term: t.term,
            times: t.times,
        })
        .collect();
    let vocab: Vec<String> = db::fetch_anki_notes(&state.knowledge)
        .await?
        .into_iter()
        .map(|n| n.vocab)
        .collect();
    let words = db::fetch_word_totals(&state.knowledge).await?;

    let stats = aggregate_kanji(
        &lines,
        &lookups,
        &vocab,
        &words,
        settings.day_rollover_hour,
        tz_offset_secs(),
    );

    Ok(Json(json!({
        "kanji": stats.kanji,
        "grades": stats.grades,
        "days": stats.days,
        "total_encounters": stats.total_encounters,
        // The threshold the tinting and the grade meters share, sent rather
        // than duplicated in JS.
        "solid_encounters": SOLID_ENCOUNTERS,
    })))
}
