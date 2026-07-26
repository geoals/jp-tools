//! Incremental tokenization of the raw line stream into per-day content-word
//! counts (`word_days`). Runs on Anki refresh; a watermark row in `settings`
//! tracks the last processed line id, so each run only touches new lines.
//!
//! Tokenization uses the mined vocab as Sudachi validation headwords: a mined
//! compound found whole in Mode C is kept whole (so it matches its card),
//! anything unrecognized is split down to finer modes.

use std::collections::{HashMap, HashSet};

use jp_core::tokenize::{SudachiTokenizer, Tokenizer, is_content_word};
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

fn tz_offset_secs() -> i64 {
    chrono::Local::now().offset().local_minus_utc() as i64
}

#[derive(Debug, serde::Serialize)]
pub struct IngestOutcome {
    pub lines: usize,
    pub words: usize,
}

pub async fn ingest_new_lines(state: &AppState) -> Result<IngestOutcome, AppError> {
    let watermark: i64 = db::get_setting_raw(&state.local, WATERMARK_KEY)
        .await?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let lines = db::fetch_lines_after(&state.knowledge, watermark).await?;
    let max_id = lines.last().map(|l| l.id);
    let Some(max_id) = max_id else {
        return Ok(IngestOutcome { lines: 0, words: 0 });
    };

    let settings = db::load_settings(&state.local).await?;
    let rollover = settings.day_rollover_hour;
    let tz = tz_offset_secs();
    let vocab: HashSet<String> = db::fetch_anki_notes(&state.knowledge)
        .await?
        .into_iter()
        .map(|n| n.vocab)
        .collect();
    let dict_path = state.sudachi_dict_path.clone();

    let n_lines = lines.len();
    // Dictionary load + tokenization are CPU-bound; keep them off the runtime.
    let counts = tokio::task::spawn_blocking(move || -> Result<_, AppError> {
        let tokenizer = SudachiTokenizer::new(&dict_path, vocab)
            .map_err(|e| AppError::Upstream(format!("sudachi: {e}")))?;
        let mut counts: HashMap<(String, String), i64> = HashMap::new();
        for line in &lines {
            let date = stats::date_key(line.ts, rollover, tz).to_string();
            match tokenizer.tokenize(&line.text) {
                Ok(tokens) => {
                    for t in tokens {
                        if is_content_word(&t.pos) {
                            *counts.entry((t.base_form, date.clone())).or_default() += 1;
                        }
                    }
                }
                Err(e) => warn!(line_id = line.id, error = %e, "tokenize failed, skipping line"),
            }
        }
        Ok(counts)
    })
    .await
    .map_err(|e| AppError::Upstream(format!("tokenize task panicked: {e}")))??;

    let rows: Vec<(String, String, i64)> = counts
        .into_iter()
        .map(|((lemma, date), count)| (lemma, date, count))
        .collect();
    db::add_word_day_counts(&state.knowledge, &rows).await?;
    db::save_setting(&state.local, WATERMARK_KEY, &max_id.to_string()).await?;

    info!(lines = n_lines, words = rows.len(), "line ingest complete");
    Ok(IngestOutcome {
        lines: n_lines,
        words: rows.len(),
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
/// `word_days` beside the hooked ones. This is deliberately *not* true of the
/// lookup rates, which stay over hooked reading only — see `stats::rate` and
/// `stats::kanji`.
pub async fn ingest_new_sessions(state: &AppState) -> Result<IngestOutcome, AppError> {
    let watermark: i64 = db::get_setting_raw(&state.local, SESSION_WATERMARK_KEY)
        .await?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let sessions = db::fetch_session_texts_after(&state.knowledge, watermark).await?;
    let Some(max_id) = sessions.last().map(|s| s.id) else {
        return Ok(IngestOutcome { lines: 0, words: 0 });
    };

    let settings = db::load_settings(&state.local).await?;
    let rollover = settings.day_rollover_hour;
    let tz = tz_offset_secs();
    let vocab: HashSet<String> = db::fetch_anki_notes(&state.knowledge)
        .await?
        .into_iter()
        .map(|n| n.vocab)
        .collect();
    let dict_path = state.sudachi_dict_path.clone();

    let n_sessions = sessions.len();
    let counts = tokio::task::spawn_blocking(move || -> Result<_, AppError> {
        let tokenizer = SudachiTokenizer::new(&dict_path, vocab)
            .map_err(|e| AppError::Upstream(format!("sudachi: {e}")))?;
        let mut counts: HashMap<(String, String), i64> = HashMap::new();
        for s in &sessions {
            let date = stats::date_key(s.start_ts, rollover, tz).to_string();
            // Sentence by sentence: Sudachi has an input length limit, and an
            // article is far longer than the hooked line this path was built
            // for. Splitting keeps every token in the same analysis window it
            // would have had as a line.
            for sentence in jp_core::text::sentences::split_sentences(&s.content) {
                match tokenizer.tokenize(&sentence) {
                    Ok(tokens) => {
                        for t in tokens {
                            if is_content_word(&t.pos) {
                                *counts.entry((t.base_form, date.clone())).or_default() += 1;
                            }
                        }
                    }
                    Err(e) => warn!(session_id = s.id, error = %e, "tokenize failed, skipping"),
                }
            }
        }
        Ok(counts)
    })
    .await
    .map_err(|e| AppError::Upstream(format!("tokenize task panicked: {e}")))??;

    let rows: Vec<(String, String, i64)> = counts
        .into_iter()
        .map(|((lemma, date), count)| (lemma, date, count))
        .collect();
    db::add_word_day_counts(&state.knowledge, &rows).await?;
    db::save_setting(&state.local, SESSION_WATERMARK_KEY, &max_id.to_string()).await?;

    info!(
        sessions = n_sessions,
        words = rows.len(),
        "session ingest complete"
    );
    Ok(IngestOutcome {
        lines: n_sessions,
        words: rows.len(),
    })
}
