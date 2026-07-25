//! "What does this line actually say" — a short read from the model.

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::{Value, json};

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
pub async fn explain_line(
    State(state): State<AppState>,
    Json(body): Json<ExplainBody>,
) -> Result<Json<Value>, AppError> {
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
    let text =
        crate::services::llm::explain(&state.http, &api_key, &context, body.focus.trim()).await?;
    Ok(Json(json!({ "text": text })))
}
