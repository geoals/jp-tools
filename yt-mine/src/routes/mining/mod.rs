use jp_core::highlight;
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
    /// The ledger's reading for the pair, so a click can ask about 空/そら
    /// rather than about 空.
    pub(crate) reading: String,
    /// Where the surface starts, in UTF-16 code units — what the popup's scan
    /// slices the line at. Counting the surfaces as they come would drift,
    /// since the tokenizer drops characters (the emphatic っ) that the line
    /// still has.
    pub(crate) start: usize,
    /// `new` / `seen` / `unknown` / `known`, or empty for a token the ledger
    /// gives no verdict on. Empty throughout in fake mode.
    pub(crate) status: String,
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
    let mut views = Vec::with_capacity(sentences.len());
    for s in sentences {
        if s.end_time > max_end {
            max_end = s.end_time;
        }
        views.push(SentenceView {
            id: s.id,
            timestamp: format_seconds(s.start_time),
            start_seconds: s.start_time as u64,
            tokens: tokens_for(state, &s.text).await,
            text: s.text,
        });
    }
    Ok((views, max_end))
}

/// One sentence's tokens, as the reader's own pipeline sees them.
///
/// `analyze` is the same call read-stats' feed makes, so a word is segmented,
/// keyed and judged identically whether it was met in a VN or in a transcript.
/// Without a highlighter — fake mode — it falls back to the bare tokenizer,
/// which gives the surfaces and no ledger verdict.
async fn tokens_for(state: &AppState, text: &str) -> Vec<TokenView> {
    let Some(h) = &state.highlighter else {
        return state
            .tokenizer
            .tokenize(text)
            .map(|toks| {
                toks.into_iter()
                    .map(|t| TokenView {
                        is_content_word: is_content_word(&t.pos),
                        base_form: t.base_form,
                        surface: t.surface,
                        reading: String::new(),
                        start: 0,
                        status: String::new(),
                    })
                    .collect()
            })
            .unwrap_or_default();
    };
    highlight::analyze(&state.knowledge, h, text)
        .await
        .into_iter()
        .map(|a| TokenView {
            // Both tests. `status` being set means `analyze` let it past the
            // wordhood gate, the name filter and the blacklist — but that gate
            // admits anything the master dictionary lists, and Sankoku lists
            // は, を and の. A particle is not a word to mine.
            is_content_word: is_content_word(&a.pos) && a.status.is_some(),
            base_form: a.headword,
            surface: a.surface,
            reading: a.judged_as.unwrap_or(a.reading),
            start: a.start,
            status: a.status.unwrap_or_default().to_string(),
        })
        .collect()
}

pub(crate) fn format_seconds(secs: f64) -> String {
    let total = secs as u64;
    let minutes = total / 60;
    let seconds = total % 60;
    format!("{minutes}:{seconds:02}")
}

#[cfg(test)]
mod tests;
