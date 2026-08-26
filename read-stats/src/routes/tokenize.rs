//! `#tokenize` — run a line of text through the reading view's pipeline and
//! show what came out.
//!
//! It exists because the pipeline is the one part of this app whose output the
//! reader cannot check: the feed shows tints and nothing else, so a term split
//! wrong, a compound never rejoined or a name mis-tagged is only visible as a
//! word that was never asked about. Pasting the line here says what Sudachi
//! made of it, in the same terms the ledger is keyed on.
//!
//! It answers with [`super::reader::highlight::analyze`] and nothing of its own — a
//! page for testing the pipeline that ran a second, subtly different pipeline
//! would be worse than no page. That is also why the excluded tokens come back
//! rather than being filtered as the feed filters them: what was dropped, and
//! under which rule, is most of the question being asked.
//!
//! It also answers *why*: `steps` is the pipeline's own trace, every decision
//! it made in the order it made them. The table says what happened; that says
//! which rule it happened under, which is the difference between checking a
//! parse against the system and having to reason it out from the outside.
//!
//! Read-only. It writes no ledger row, no `word_days` count and no presence
//! mark — text pasted here was not read.

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::AppError;
use crate::routes::reader::highlight;

#[derive(Deserialize)]
pub struct TokenizeBody {
    pub text: String,
}

/// A paragraph's worth. The pipeline is happy with more, but the page shows
/// every token as its own row, and the cost of a paste of a whole chapter is
/// paid by the browser rendering thousands of them.
const MAX_TEXT_CHARS: usize = 2000;

pub async fn tokenize_text(
    State(state): State<AppState>,
    Json(body): Json<TokenizeBody>,
) -> Result<Json<Value>, AppError> {
    let text = body.text.trim().to_string();
    if text.is_empty() {
        return Err(AppError::BadRequest("no text to tokenize".into()));
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(AppError::BadRequest(format!(
            "text is longer than {MAX_TEXT_CHARS} characters"
        )));
    }
    let Some(h) = highlight::shared(&state).await else {
        return Err(AppError::Upstream(
            "tokenizer unavailable — check the Sudachi dictionary path".into(),
        ));
    };
    let tokens = highlight::analyze(&state.knowledge, &h, &text).await;
    let steps = h.explain(&text);
    Ok(Json(
        json!({ "text": text, "tokens": tokens, "steps": steps }),
    ))
}
