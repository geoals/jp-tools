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
//! The note type and its field names come from [`jp_mine_core::config::AnkiConfig`],
//! the same map the exporter uses, so this card fits whichever of the two
//! supported note types is configured.
//!
//! The pitch fields are rebuilt here rather than left empty, because the card
//! template needs them: `markPitch()` colours the target word by the first
//! digit it finds in the pitch-position field, so an empty one silently costs
//! the colour. The markup is Yomitan's own, reproduced span for span.
//!
//! The word audio is a native recording from the local-audio add-on, which is
//! also where Yomitan's audio sources point, so both surfaces attach the same
//! file. The sentence audio and picture come from vn-capture, exactly as they
//! do for a Yomitan card.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use jp_core::knowledge::dictionaries;
use jp_mine_core::card::{self, bold_surface, furigana, pitch_num, pitch_pattern};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::AppError;

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
    let frequency = match dictionaries::reader_frequency(pool).await? {
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

    // The note type and its field names come from `AnkiConfig`, so this card is
    // the shape the configured note type actually has. A field the note type
    // does not carry is `None` there and is left out rather than sent and
    // refused.
    let anki = &state.anki;
    let mut fields = serde_json::Map::new();
    let mut put = |name: &Option<String>, value: String| {
        if let Some(name) = name {
            fields.insert(name.clone(), Value::String(value));
        }
    };
    put(&anki.field_vocab, req.term.clone());
    put(&anki.field_reading, req.reading.clone());
    put(&anki.field_furigana, furigana(&req.term, &req.reading));
    put(&anki.field_definition, glossary);
    put(
        &anki.field_sentence,
        bold_surface(&req.sentence, &req.surface),
    );
    put(&anki.field_source, settings.current_work.clone());
    put(
        &anki.field_frequency,
        frequency.map(|f| f.to_string()).unwrap_or_default(),
    );
    put(
        &anki.field_freq_sort,
        frequency.map(|f| f.to_string()).unwrap_or_default(),
    );
    put(&anki.field_vocab_audio, vocab_audio);
    put(
        &anki.field_pitch_num,
        accent.map(pitch_num).unwrap_or_default(),
    );
    put(
        &anki.field_pitch_pattern,
        accent
            .map(|a| pitch_pattern(&req.reading, a))
            .unwrap_or_default(),
    );

    let note = json!({
        "action": "addNote",
        "version": 6,
        "params": { "note": {
            "deckName": anki.deck_name,
            "modelName": anki.model_name,
            "fields": fields,
            "tags": [TAG],
            "options": { "allowDuplicate": false },
        }},
    });

    let body =
        Bytes::from(serde_json::to_vec(&note).map_err(|e| AppError::Upstream(e.to_string()))?);
    let (_status, replied) = crate::services::card::add_note(&state, body)
        .await
        .map_err(AppError::Upstream)?;
    // The id comes back so the open popup can raise its mined badge without
    // asking Anki a second time — and a duplicate answers `null`, which is the
    // honest answer to "did this add a card".
    let note_id = crate::services::card::new_note_id(&replied);
    // AnkiConnect answers 200 with the refusal in the body, so the status code is
    // not the outcome: a missing note type reads as a success with no card behind
    // it. `ok` is whether a card exists now, and `error` is Anki's own sentence.
    let error = anki_error(&replied);
    Ok(Json(
        json!({ "ok": note_id.is_some(), "note_id": note_id, "error": error }),
    ))
}

/// Anki's refusal, in its own words. Empty and null both mean it did not
/// refuse; an `.apkg` import returns an empty string for a real failure, but an
/// `addNote` does not.
fn anki_error(replied: &Bytes) -> Option<String> {
    let json: Value = serde_json::from_slice(replied).ok()?;
    let text = json.get("error")?.as_str()?;
    (!text.is_empty()).then(|| text.to_string())
}
