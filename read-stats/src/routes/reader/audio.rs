//! Word audio, from the Local Audio Server running beside Anki.
//!
//! Two routes because the page needs two things and only one of them is JSON:
//! what clips exist, and one clip's bytes. Both proxy — see
//! [`crate::services::audio`] for why the page cannot ask that server itself.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::AppError;
use crate::services::audio;

#[derive(Deserialize)]
pub struct AudioQuery {
    /// The headword, spelt as the dictionary spells it.
    pub term: String,
    /// Narrows to one pronunciation. The audio server matches on the pair, so
    /// 空 does not come back read から when the line meant そら.
    #[serde(default)]
    pub reading: String,
}

/// `GET /api/reader/audio?term=&reading=` — what there is to play, best first.
///
/// Never an error: no server, no clips and an unreadable answer are all "no
/// audio for this word", which is the ordinary case and draws no button.
pub async fn audio(State(state): State<AppState>, Query(q): Query<AudioQuery>) -> Json<Value> {
    let sources = audio::sources(&state, &q.term, &q.reading).await;
    Json(json!({ "sources": sources }))
}

#[derive(Deserialize)]
pub struct ClipQuery {
    /// A path on the audio server, exactly as a [`audio::Source`] gave it.
    pub path: String,
}

/// `GET /api/reader/audio/clip?path=` — one clip, proxied whole.
///
/// Whole rather than streamed: a word's pronunciation is tens of kilobytes, and
/// the play must start on the click rather than after a first chunk.
pub async fn clip(
    State(state): State<AppState>,
    Query(q): Query<ClipQuery>,
) -> Result<Response, AppError> {
    let (content_type, bytes) = audio::fetch_clip(&state, &q.path).await?;
    Ok(([(CONTENT_TYPE, content_type)], bytes).into_response())
}
