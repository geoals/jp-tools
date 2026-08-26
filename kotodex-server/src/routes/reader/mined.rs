//! Whether a word is already a card, and the way to it.
//!
//! Asked of Anki rather than of `anki_notes`: that table is a snapshot taken on
//! demand, and the case that matters most here is a card made seconds ago. It
//! is the same duplicate check Yomitan runs before offering to add, so the
//! popup's badge means what Yomitan's does.
//!
//! Anki being closed is not an error — the badge simply does not appear.

use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::app::AppState;
use crate::error::AppError;
use crate::services::anki;

#[derive(Deserialize)]
pub struct MinedQuery {
    /// The word as the card would spell it — what `mine` writes into the vocab
    /// field, so this asks exactly "would mining this be a duplicate?".
    pub term: String,
}

#[derive(Serialize)]
pub struct Mined {
    /// Also the card's creation time, in epoch milliseconds.
    pub note_id: Option<i64>,
}

/// `GET /api/reader/mined?term=<term>`
pub async fn mined(State(state): State<AppState>, Query(q): Query<MinedQuery>) -> Json<Mined> {
    let note_id = match anki::find_note_for_vocab(
        &state.http,
        &state.anki_url,
        &state.anki_vocab_field,
        &q.term,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            debug!(error = %e, term = %q.term, "no answer from Anki for the mined badge");
            None
        }
    };
    Json(Mined { note_id })
}

#[derive(Deserialize)]
pub struct BrowseRequest {
    pub note_id: i64,
}

/// `POST /api/reader/mined/browse` — raise Anki's card browser on the note.
pub async fn browse(
    State(state): State<AppState>,
    Json(req): Json<BrowseRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    anki::gui_browse(&state.http, &state.anki_url, req.note_id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
