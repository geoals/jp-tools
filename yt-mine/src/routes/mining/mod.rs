use jp_core::tokenize::is_content_word;

use crate::app::AppState;
use crate::db;
use crate::error::AppError;

// --- Shared view types ---

pub(crate) struct SentenceView {
    pub(crate) id: i64,
    pub(crate) timestamp: String,
    pub(crate) start_seconds: u64,
    pub(crate) tokens: Vec<TokenView>,
    pub(crate) text: String,
}

pub(crate) struct TokenView {
    pub(crate) surface: String,
    pub(crate) base_form: String,
    pub(crate) is_content_word: bool,
}

// --- Shared business logic ---

/// Build sentence views for display. Always tokenizes sentences so they are
/// interactive as soon as they appear (even during transcription).
///
/// Returns `(views, max_end_time)` where `max_end_time` is the highest
/// `end_time` across all sentences (0.0 if none). Used to compute progress %.
pub(crate) async fn build_sentence_views(
    state: &AppState,
    job_id: i64,
) -> Result<(Vec<SentenceView>, f64), AppError> {
    let sentences = db::get_sentences_for_job(&state.db, job_id).await?;
    let mut max_end: f64 = 0.0;
    let views = sentences
        .into_iter()
        .map(|s| {
            if s.end_time > max_end {
                max_end = s.end_time;
            }
            let tokens = state
                .tokenizer
                .tokenize(&s.text)
                .map(|toks| {
                    toks.into_iter()
                        .map(|t| TokenView {
                            is_content_word: is_content_word(&t.pos),
                            base_form: t.base_form,
                            surface: t.surface,
                        })
                        .collect()
                })
                .unwrap_or_default();
            SentenceView {
                id: s.id,
                timestamp: format_seconds(s.start_time),
                start_seconds: s.start_time as u64,
                text: s.text,
                tokens,
            }
        })
        .collect();
    Ok((views, max_end))
}

pub(crate) fn format_seconds(secs: f64) -> String {
    let total = secs as u64;
    let minutes = total / 60;
    let seconds = total % 60;
    format!("{minutes}:{seconds:02}")
}

#[cfg(test)]
mod tests;
