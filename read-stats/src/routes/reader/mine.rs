//! Making a card from the overlay, the way Yomitan makes one.
//!
//! The note is built here and then handed to
//! [`crate::services::card::add_note`], which every card path calls — Yomitan's
//! own add arrives there too, through the proxy. That is deliberate and
//! load-bearing: note-id extraction, the CompactDef enrichment, vn-capture's
//! screenshot and voiceline, the deck mirror and the notification all hang off that
//! one function, so an overlay card and a Yomitan card are the same thing
//! downstream rather than a second implementation kept in step by hand.
//!
//! The pitch fields are rebuilt here rather than left empty, because the card
//! template needs them: `markPitch()` colours the target word by the first
//! digit it finds in `VocabPitchNum`, so an empty field silently costs the
//! colour. The markup is Yomitan's own, reproduced span for span.
//!
//! `VocabAudio` is a native recording from the local-audio add-on, which is
//! also where Yomitan's audio sources point, so both surfaces attach the same
//! file. `SentAudio` and `Image` come from vn-capture, exactly as they do for a
//! Yomitan card.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use jp_core::knowledge::dictionaries;
use jp_mine_core::card::{self, bold_surface, furigana, pitch_num, pitch_pattern};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::AppError;
use crate::ingest::READER_FREQUENCY;

/// The note type every mined card uses. Not a setting because the field names
/// below are not either — they are one shape, and half of it configurable
/// would only let the two halves disagree.
const MODEL: &str = "Japanese sentences";

/// Marks the card as coming from the sentence-mining flow, matching what
/// Yomitan tags its own with, so a search for one finds both.
const TAG: &str = "yomitan";

#[derive(Deserialize)]
pub struct MineRequest {
    /// The ledger headword — what goes in the vocabulary field.
    pub term: String,
    pub reading: String,
    /// The word as the line spelt it, which is what gets bolded in the
    /// sentence. 振る is the card's word; 振っ is what to find in the text.
    pub surface: String,
    pub sentence: String,
}

/// `POST /api/reader/mine`
pub async fn mine(
    State(state): State<AppState>,
    Json(req): Json<MineRequest>,
) -> Result<Json<Value>, AppError> {
    let pool = state.knowledge.pool();
    let settings = crate::db::load_settings(&state.local).await?;

    // The rank the reader's own underline uses, so the card agrees with the
    // page it was mined from.
    let frequency = match dictionaries::by_title(pool, READER_FREQUENCY).await? {
        Some(d) => dictionaries::lookup_frequency(pool, d.id, &req.term).await?,
        None => None,
    };

    let glossary = card::glossary(pool, &req.term, &req.reading).await?;
    let accent = card::accent(pool, &req.term, &req.reading).await?;
    // Before the add, not after: the audio is a field of the note, and writing
    // it afterwards would be a second write to race the editor with.
    let vocab_audio = crate::services::anki::store_vocab_audio(
        &state.http,
        &state.anki_url,
        &req.term,
        &req.reading,
    )
    .await;

    let note = json!({
        "action": "addNote",
        "version": 6,
        "params": { "note": {
            "deckName": state.anki_deck,
            "modelName": MODEL,
            "fields": {
                state.anki_vocab_field.clone(): req.term,
                "VocabFurigana": furigana(&req.term, &req.reading),
                "VocabDefFull": glossary,
                state.anki_sentence_field.clone(): bold_surface(&req.sentence, &req.surface),
                "Document": settings.current_work,
                "Frequency": frequency.map(|f| f.to_string()).unwrap_or_default(),
                "VocabAudio": vocab_audio,
                "VocabPitchNum": accent.map(pitch_num).unwrap_or_default(),
                "VocabPitchPattern": accent
                    .map(|a| pitch_pattern(&req.reading, a))
                    .unwrap_or_default(),
            },
            "tags": [TAG],
            "options": { "allowDuplicate": false },
        }},
    });

    let body =
        Bytes::from(serde_json::to_vec(&note).map_err(|e| AppError::Upstream(e.to_string()))?);
    let (status, replied) = crate::services::card::add_note(&state, body)
        .await
        .map_err(AppError::Upstream)?;
    let ok = status.is_success();
    // The id comes back so the open popup can raise its mined badge without
    // asking Anki a second time — and a duplicate answers `null`, which is the
    // honest answer to "did this add a card".
    let note_id = crate::services::card::new_note_id(&replied);
    Ok(Json(json!({ "ok": ok, "note_id": note_id })))
}
