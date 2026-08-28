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

/// What the surfaces test for to know the answer is "set a key", not "it broke".
/// A sentinel rather than a status code: the reader shows the message on every
/// other failure, and this is the one that has somewhere to go instead.
pub const NO_KEY: &str = "NO_KEY";

/// Ask the model for a short read on the line currently being read, centred on
/// a selected word if one was passed.
///
/// Answers as server-sent events — a `delta` per piece of text, then `done`, or
/// a single `error`. The reader is mid-line while this runs, so the first words
/// arriving beats the whole answer arriving a few seconds later. A failure
/// before the stream opens is still an ordinary HTTP error; one after it can
/// only be an event, since the status line is long gone.
///
/// No key is `NO_KEY`, which the reading surfaces answer by opening the field to
/// paste one into. Reaching a button and being told the install is misconfigured
/// is the state this exists to end.
pub async fn explain_line(
    State(state): State<AppState>,
    Json(body): Json<ExplainBody>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, AppError> {
    let Some(provider) = crate::services::llm::provider(&state).await? else {
        return Err(AppError::BadRequest(NO_KEY.into()));
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
        crate::services::llm::explain_stream(&state.http, &provider, &context, body.focus.trim());

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
