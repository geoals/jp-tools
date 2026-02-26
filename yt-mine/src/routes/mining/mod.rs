use std::sync::Arc;

use crate::app::AppState;
use crate::db;
use crate::error::AppError;
use crate::services::dictionary::Dictionary;
use crate::services::tokenize::{Token, is_content_word};

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

pub(crate) struct WordLookupResult {
    pub(crate) definition_html: Option<String>,
    pub(crate) reading: String,
    pub(crate) pitch_num: Option<String>,
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

pub(crate) async fn lookup_word(dictionaries: &[Arc<Dictionary>], word: &str) -> WordLookupResult {
    let mut def_parts = Vec::new();
    let mut reading = String::new();
    let mut pitch_num = None;

    for dict in dictionaries {
        let entries = dict.lookup(word).await;
        if let Some(entry) = entries.first() {
            let joined = entry.definitions.join("; ");
            def_parts.push(dict.wrap_definitions(&joined));
            if reading.is_empty() && !entry.reading.is_empty() {
                reading = entry.reading.clone();
            }
        }
        if pitch_num.is_none() {
            let pitch = dict.lookup_pitch(word).await;
            if !pitch.is_empty() {
                let nums: Vec<String> = pitch[0]
                    .positions
                    .iter()
                    .map(|p| p.to_string())
                    .collect();
                pitch_num = Some(nums.join(","));
            }
        }
    }

    WordLookupResult {
        definition_html: if def_parts.is_empty() {
            None
        } else {
            Some(def_parts.join(""))
        },
        reading,
        pitch_num,
    }
}

/// Build sentence HTML with the target word's surface form(s) wrapped in `<b></b>`.
pub(crate) fn bold_target_in_sentence(tokens: &[Token], target_base_form: &str) -> Option<String> {
    if !tokens.iter().any(|t| t.base_form == target_base_form) {
        return None;
    }
    let mut result = String::new();
    for token in tokens {
        if token.base_form == target_base_form {
            result.push_str("<b>");
            result.push_str(&token.surface);
            result.push_str("</b>");
        } else {
            result.push_str(&token.surface);
        }
    }
    Some(result)
}

pub(crate) fn format_seconds(secs: f64) -> String {
    let total = secs as u64;
    let minutes = total / 60;
    let seconds = total % 60;
    format!("{minutes}:{seconds:02}")
}

#[cfg(test)]
mod tests;
