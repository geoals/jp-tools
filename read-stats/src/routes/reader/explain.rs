//! "What does this line actually say" — a short read from the model.

use std::convert::Infallible;

use axum::Json;
use axum::extract::State;
use axum::response::Sse;
use axum::response::sse::Event;
use serde::Deserialize;
use serde_json::json;

use crate::app::AppState;
use crate::error::AppError;

/// Recent lines and, optionally, the word the reader has selected in the last
/// one. The lines are oldest-first with the target line last, mirroring what
/// the feed shows on screen so the server never has to guess which line is "the
/// current one".
#[derive(Deserialize)]
pub struct ExplainBody {
    pub context: Vec<String>,
    #[serde(default)]
    pub focus: String,
}

/// Enough earlier lines to place a pronoun or an unstated subject without
/// paying for a whole scene. The client sends what is on screen; this caps it.
const MAX_EXPLAIN_CONTEXT: usize = 12;

/// Ask the model for a short read on the line currently being read, centred on
/// a selected word if one was passed. Off unless an API key is configured.
///
/// Answers as server-sent events — a `delta` per piece of text, then `done`, or
/// a single `error`. The reader is mid-line while this runs, so the first words
/// arriving beats the whole answer arriving a few seconds later. A failure
/// before the stream opens is still an ordinary HTTP error; one after it can
/// only be an event, since the status line is long gone.
pub async fn explain_line(
    State(state): State<AppState>,
    Json(body): Json<ExplainBody>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, AppError> {
    let Some(api_key) = state.anthropic_api_key.clone() else {
        return Err(AppError::BadRequest(
            "no Anthropic API key set (JP_TOOLS_ANTHROPIC_API_KEY)".into(),
        ));
    };

    let mut context: Vec<String> = body
        .context
        .into_iter()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if context.is_empty() {
        return Err(AppError::BadRequest("no line to explain".into()));
    }
    if context.len() > MAX_EXPLAIN_CONTEXT {
        context.drain(0..context.len() - MAX_EXPLAIN_CONTEXT);
    }

    super::mark_presence(&state, "explain").await;
    let deltas =
        crate::services::llm::explain_stream(&state.http, &api_key, &context, body.focus.trim());

    Ok(Sse::new(async_stream::stream! {
        let mut deltas = std::pin::pin!(deltas);
        while let Some(delta) = futures_util::StreamExt::next(&mut deltas).await {
            match delta {
                // JSON rather than the raw text: a delta can contain a newline,
                // which is the one thing an SSE `data:` field cannot.
                Ok(text) => yield Ok(Event::default().event("delta").data(json!(text).to_string())),
                Err(e) => {
                    yield Ok(Event::default().event("error").data(e.to_string()));
                    return;
                }
            }
        }
        yield Ok(Event::default().event("done").data(""));
    }))
}
