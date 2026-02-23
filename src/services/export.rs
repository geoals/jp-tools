use std::future::Future;
use std::pin::Pin;

use serde_json::{json, Map, Value};
use tokio::fs;
use tracing::{debug, info, warn};

use crate::config::AnkiConfig;
use crate::models::Sentence;

/// A sentence bundled with optional media file paths for Anki export.
#[derive(Debug, Clone)]
pub struct ExportSentence {
    pub sentence: Sentence,
    /// Absolute path to the screenshot jpg on disk (if extraction succeeded).
    pub screenshot_path: Option<String>,
    /// Absolute path to the audio clip mp3 on disk (if extraction succeeded).
    pub audio_clip_path: Option<String>,
    /// The target vocabulary word selected by the user (base/dictionary form).
    pub target_word: Option<String>,
    /// Dictionary definition of the target word, if found.
    pub definition: Option<String>,
    /// Anki bracket furigana notation, e.g. `隔週[かくしゅう]`.
    pub vocab_furigana: Option<String>,
    /// Pitch accent downstep position(s), e.g. `"0"` or `"0,3"`.
    pub vocab_pitch_num: Option<String>,
    /// Sentence text with the target word wrapped in `<b></b>` for Anki display.
    pub sentence_html: Option<String>,
}

#[cfg_attr(test, mockall::automock)]
pub trait AnkiExporter: Send + Sync {
    fn export_sentences(
        &self,
        sentences: Vec<ExportSentence>,
        source: String,
    ) -> Pin<Box<dyn Future<Output = Result<usize, ExportError>> + Send>>;
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("export failed: {0}")]
    Failed(String),
}

/// Data for a single Anki note, ready to be serialized.
pub struct NoteData {
    pub sentence_text: String,
    pub vocab_kanji: String,
    pub vocab_def: String,
    pub source: String,
    pub screenshot_filename: Option<String>,
    pub audio_clip_filename: Option<String>,
    pub vocab_furigana: String,
    pub vocab_pitch_num: String,
}

/// Build the AnkiConnect `addNotes` request body using the configured field mapping.
/// Only fields that are mapped (Some) in the config are included on the note.
pub fn build_add_notes_request(notes: &[NoteData], config: &AnkiConfig) -> Value {
    let notes_json: Vec<Value> = notes
        .iter()
        .map(|n| {
            let mut fields = Map::new();

            if let Some(ref f) = config.field_vocab {
                fields.insert(f.clone(), json!(n.vocab_kanji));
            }
            if let Some(ref f) = config.field_definition {
                fields.insert(f.clone(), json!(n.vocab_def));
            }
            if let Some(ref f) = config.field_sentence {
                fields.insert(f.clone(), json!(n.sentence_text));
            }
            if let Some(ref f) = config.field_image {
                let val = n
                    .screenshot_filename
                    .as_deref()
                    .map(|name| format!("<img src=\"{name}\">"))
                    .unwrap_or_default();
                fields.insert(f.clone(), json!(val));
            }
            if let Some(ref f) = config.field_audio {
                let val = n
                    .audio_clip_filename
                    .as_deref()
                    .map(|name| format!("[sound:{name}]"))
                    .unwrap_or_default();
                fields.insert(f.clone(), json!(val));
            }
            if let Some(ref f) = config.field_source {
                fields.insert(f.clone(), json!(n.source));
            }
            if let Some(ref f) = config.field_furigana {
                fields.insert(f.clone(), json!(n.vocab_furigana));
            }
            if let Some(ref f) = config.field_pitch_num {
                fields.insert(f.clone(), json!(n.vocab_pitch_num));
            }

            json!({
                "deckName": config.deck_name,
                "modelName": config.model_name,
                "fields": fields,
                "tags": ["jp-tools", "youtube"]
            })
        })
        .collect();

    json!({
        "action": "addNotes",
        "version": 6,
        "params": {
            "notes": notes_json
        }
    })
}

fn format_timestamp(secs: f64) -> String {
    let total = secs as u64;
    let minutes = total / 60;
    let seconds = total % 60;
    format!("{minutes}:{seconds:02}")
}

/// Build the request to create a basic fallback note type.
/// Only includes fields that are mapped in the config.
fn build_create_model_request(config: &AnkiConfig) -> Value {
    let fields: Vec<&str> = [
        config.field_vocab.as_deref(),
        config.field_definition.as_deref(),
        config.field_sentence.as_deref(),
        config.field_image.as_deref(),
        config.field_audio.as_deref(),
        config.field_source.as_deref(),
        config.field_furigana.as_deref(),
        config.field_pitch_num.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect();

    // Build a minimal card template from the first field (sentence if available)
    let front = config
        .field_sentence
        .as_ref()
        .map(|f| format!("{{{{{f}}}}}"))
        .unwrap_or_else(|| "{{Front}}".into());

    let back_parts: Vec<String> = fields.iter().map(|f| format!("{{{{{f}}}}}")).collect();
    let back = back_parts.join("<br>");

    json!({
        "action": "createModel",
        "version": 6,
        "params": {
            "modelName": config.model_name,
            "inOrderFields": fields,
            "cardTemplates": [{
                "Name": "Card 1",
                "Front": front,
                "Back": back
            }]
        }
    })
}

/// Build the request to create the deck if it doesn't exist.
fn build_create_deck_request(deck_name: &str) -> Value {
    json!({
        "action": "createDeck",
        "version": 6,
        "params": {
            "deck": deck_name
        }
    })
}

/// Check whether a model (note type) already exists in Anki.
fn build_model_names_request() -> Value {
    json!({
        "action": "modelNames",
        "version": 6
    })
}

/// Build a `storeMediaFile` request for AnkiConnect.
/// Uses the `data` field (base64-encoded bytes) so it works even when
/// AnkiConnect runs on a different machine than jp-tools.
pub fn build_store_media_request(filename: &str, data_base64: &str) -> Value {
    json!({
        "action": "storeMediaFile",
        "version": 6,
        "params": {
            "filename": filename,
            "data": data_base64
        }
    })
}

/// Read a file and return its contents as a base64-encoded string.
async fn read_file_base64(path: &str) -> Result<String, ExportError> {
    use base64::Engine;
    let bytes = fs::read(path)
        .await
        .map_err(|e| ExportError::Failed(format!("failed to read media file {path}: {e}")))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

pub struct AnkiConnectExporter {
    pub anki_url: String,
    pub config: AnkiConfig,
    client: reqwest::Client,
}

impl AnkiConnectExporter {
    pub fn new(anki_url: String, config: AnkiConfig) -> Self {
        Self {
            anki_url,
            config,
            client: reqwest::Client::builder()
                .pool_max_idle_per_host(0)
                .build()
                .expect("failed to build HTTP client"),
        }
    }
}

/// Send a request to AnkiConnect and check for errors in the response.
async fn send_anki_request(
    client: &reqwest::Client,
    url: &str,
    body: &Value,
) -> Result<Value, ExportError> {
    let response = client
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|e| ExportError::Failed(format!("AnkiConnect request failed: {e:?}")))?;

    let body: Value = response
        .json()
        .await
        .map_err(|e| ExportError::Failed(format!("failed to parse response: {e}")))?;

    // AnkiConnect may return errors as a string or as a non-null JSON value
    match body.get("error") {
        Some(Value::String(error)) => {
            return Err(ExportError::Failed(format!("AnkiConnect error: {error}")));
        }
        Some(Value::Null) | None => {}
        Some(other) => {
            return Err(ExportError::Failed(format!("AnkiConnect error: {other}")));
        }
    }

    Ok(body)
}

/// Check if a model already exists in Anki by querying modelNames.
async fn model_exists(
    client: &reqwest::Client,
    url: &str,
    model_name: &str,
) -> Result<bool, ExportError> {
    let resp = send_anki_request(client, url, &build_model_names_request()).await?;
    let names = resp["result"]
        .as_array()
        .ok_or_else(|| ExportError::Failed("unexpected modelNames response".into()))?;
    Ok(names.iter().any(|v| v.as_str() == Some(model_name)))
}

impl AnkiExporter for AnkiConnectExporter {
    fn export_sentences(
        &self,
        sentences: Vec<ExportSentence>,
        source: String,
    ) -> Pin<Box<dyn Future<Output = Result<usize, ExportError>> + Send>> {
        let client = self.client.clone();
        let anki_url = self.anki_url.clone();
        let config = self.config.clone();
        let count = sentences.len();

        Box::pin(async move {
            // Only create the model if it doesn't already exist
            if !model_exists(&client, &anki_url, &config.model_name).await? {
                info!(model = %config.model_name, "model not found, creating fallback");
                send_anki_request(&client, &anki_url, &build_create_model_request(&config))
                    .await?;
            }

            send_anki_request(
                &client,
                &anki_url,
                &build_create_deck_request(&config.deck_name),
            )
            .await?;

            // Upload media files (read bytes and send as base64)
            for es in &sentences {
                if config.field_image.is_some() {
                    if let Some(screenshot_path) = &es.screenshot_path {
                        let filename = std::path::Path::new(screenshot_path)
                            .file_name()
                            .and_then(|f| f.to_str())
                            .unwrap_or("screenshot.jpg");
                        match read_file_base64(screenshot_path).await {
                            Ok(data) => {
                                let req = build_store_media_request(filename, &data);
                                send_anki_request(&client, &anki_url, &req).await?;
                            }
                            Err(e) => warn!(path = screenshot_path, error = %e, "skipping screenshot upload"),
                        }
                    }
                }

                if config.field_audio.is_some() {
                    if let Some(audio_clip_path) = &es.audio_clip_path {
                        let filename = std::path::Path::new(audio_clip_path)
                            .file_name()
                            .and_then(|f| f.to_str())
                            .unwrap_or("clip.mp3");
                        match read_file_base64(audio_clip_path).await {
                            Ok(data) => {
                                let req = build_store_media_request(filename, &data);
                                send_anki_request(&client, &anki_url, &req).await?;
                            }
                            Err(e) => warn!(path = audio_clip_path, error = %e, "skipping audio clip upload"),
                        }
                    }
                }
            }

            // Build note data
            let note_data: Vec<NoteData> = sentences
                .iter()
                .map(|es| {
                    let timestamp = format_timestamp(es.sentence.start_time);
                    let vocab_kanji = es
                        .target_word
                        .clone()
                        .unwrap_or_else(|| es.sentence.text.clone());
                    let vocab_def = es.definition.clone().unwrap_or_default();
                    NoteData {
                        sentence_text: es.sentence_html.clone().unwrap_or_else(|| es.sentence.text.clone()),
                        vocab_kanji,
                        vocab_def,
                        source: format!("{source} ({timestamp})"),
                        screenshot_filename: es.screenshot_path.as_ref().and_then(|p| {
                            std::path::Path::new(p)
                                .file_name()
                                .and_then(|f| f.to_str())
                                .map(|s| s.to_owned())
                        }),
                        audio_clip_filename: es.audio_clip_path.as_ref().and_then(|p| {
                            std::path::Path::new(p)
                                .file_name()
                                .and_then(|f| f.to_str())
                                .map(|s| s.to_owned())
                        }),
                        vocab_furigana: es.vocab_furigana.clone().unwrap_or_default(),
                        vocab_pitch_num: es.vocab_pitch_num.clone().unwrap_or_default(),
                    }
                })
                .collect();

            // Add notes
            let add_notes_req = build_add_notes_request(&note_data, &config);
            debug!(request = %add_notes_req, "sending addNotes to AnkiConnect");
            send_anki_request(&client, &anki_url, &add_notes_req).await?;

            Ok(count)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> AnkiConfig {
        AnkiConfig::default()
    }

    #[test]
    fn build_add_notes_request_structure() {
        let config = default_config();
        let notes = vec![
            NoteData {
                sentence_text: "テスト文".into(),
                vocab_kanji: "テスト".into(),
                vocab_def: "test".into(),
                source: "Test Video (0:05)".into(),
                screenshot_filename: Some("jp-tools_1_1.jpg".into()),
                audio_clip_filename: Some("jp-tools_1_1.mp3".into()),
                vocab_furigana: "テスト".into(),
                vocab_pitch_num: "0".into(),
            },
            NoteData {
                sentence_text: "もう一つ".into(),
                vocab_kanji: "もう一つ".into(),
                vocab_def: "".into(),
                source: "Test Video (1:05)".into(),
                screenshot_filename: None,
                audio_clip_filename: None,
                vocab_furigana: "".into(),
                vocab_pitch_num: "".into(),
            },
        ];

        let request = build_add_notes_request(&notes, &config);

        assert_eq!(request["action"], "addNotes");
        assert_eq!(request["version"], 6);

        let result_notes = request["params"]["notes"].as_array().unwrap();
        assert_eq!(result_notes.len(), 2);

        assert_eq!(result_notes[0]["deckName"], "Japanese");
        assert_eq!(result_notes[0]["modelName"], "Japanese sentences");
        assert_eq!(result_notes[0]["fields"]["SentKanji"], "テスト文");
        assert_eq!(
            result_notes[0]["fields"]["Image"],
            "<img src=\"jp-tools_1_1.jpg\">"
        );
        assert_eq!(
            result_notes[0]["fields"]["SentAudio"],
            "[sound:jp-tools_1_1.mp3]"
        );
        assert_eq!(result_notes[0]["fields"]["Document"], "Test Video (0:05)");
        assert_eq!(result_notes[0]["fields"]["VocabKanji"], "テスト");
        assert_eq!(result_notes[0]["fields"]["VocabDef"], "test");
        assert_eq!(result_notes[0]["fields"]["VocabFurigana"], "テスト");
        assert_eq!(result_notes[0]["fields"]["VocabPitchNum"], "0");

        // Second note: no media, no target word
        assert_eq!(result_notes[1]["fields"]["Image"], "");
        assert_eq!(result_notes[1]["fields"]["SentAudio"], "");
        assert_eq!(result_notes[1]["fields"]["VocabKanji"], "もう一つ");
        assert_eq!(result_notes[1]["fields"]["VocabDef"], "");
        assert_eq!(result_notes[1]["fields"]["VocabFurigana"], "");
        assert_eq!(result_notes[1]["fields"]["VocabPitchNum"], "");
    }

    #[test]
    fn build_add_notes_skips_unmapped_fields() {
        let config = AnkiConfig {
            model_name: "Minimal".into(),
            deck_name: "Test".into(),
            field_vocab: Some("Word".into()),
            field_sentence: Some("Sentence".into()),
            field_definition: None,
            field_image: None,
            field_audio: None,
            field_source: None,
            field_furigana: None,
            field_pitch_num: None,
        };

        let notes = vec![NoteData {
            sentence_text: "テスト".into(),
            vocab_kanji: "テスト".into(),
            vocab_def: "test".into(),
            source: "src".into(),
            screenshot_filename: Some("img.jpg".into()),
            audio_clip_filename: Some("clip.mp3".into()),
            vocab_furigana: "テスト".into(),
            vocab_pitch_num: "".into(),
        }];

        let request = build_add_notes_request(&notes, &config);
        let fields = &request["params"]["notes"][0]["fields"];

        assert_eq!(fields["Word"], "テスト");
        assert_eq!(fields["Sentence"], "テスト");
        assert!(fields.get("VocabDef").is_none());
        assert!(fields.get("Image").is_none());
        assert!(fields.get("SentAudio").is_none());
        assert!(fields.get("Document").is_none());
        assert!(fields.get("VocabFurigana").is_none());
        assert!(fields.get("VocabPitchNum").is_none());
    }

    #[test]
    fn build_add_notes_custom_field_names() {
        let config = AnkiConfig {
            model_name: "Custom".into(),
            deck_name: "MyDeck".into(),
            field_vocab: Some("Expression".into()),
            field_definition: Some("Meaning".into()),
            field_sentence: Some("Context".into()),
            field_image: Some("Screenshot".into()),
            field_audio: Some("Audio".into()),
            field_source: Some("Origin".into()),
            field_furigana: Some("Furigana".into()),
            field_pitch_num: Some("PitchNum".into()),
        };

        let notes = vec![NoteData {
            sentence_text: "文".into(),
            vocab_kanji: "語".into(),
            vocab_def: "def".into(),
            source: "src".into(),
            screenshot_filename: None,
            audio_clip_filename: None,
            vocab_furigana: "語[ご]".into(),
            vocab_pitch_num: "1".into(),
        }];

        let request = build_add_notes_request(&notes, &config);
        let note = &request["params"]["notes"][0];

        assert_eq!(note["modelName"], "Custom");
        assert_eq!(note["deckName"], "MyDeck");
        assert_eq!(note["fields"]["Expression"], "語");
        assert_eq!(note["fields"]["Meaning"], "def");
        assert_eq!(note["fields"]["Context"], "文");
        assert_eq!(note["fields"]["Furigana"], "語[ご]");
        assert_eq!(note["fields"]["PitchNum"], "1");
    }

    #[test]
    fn build_store_media_request_structure() {
        let req = build_store_media_request("test.jpg", "aGVsbG8=");
        assert_eq!(req["action"], "storeMediaFile");
        assert_eq!(req["params"]["filename"], "test.jpg");
        assert_eq!(req["params"]["data"], "aGVsbG8=");
        assert!(req["params"].get("path").is_none());
    }

    #[tokio::test]
    #[ignore = "requires Anki + AnkiConnect running"]
    async fn anki_connect_integration() {
        let exporter =
            AnkiConnectExporter::new("http://localhost:8765".into(), AnkiConfig::default());

        let sentences = vec![ExportSentence {
            sentence: Sentence {
                id: 1,
                job_id: 1,
                text: "テスト文です".into(),
                start_time: 0.0,
                end_time: 3.0,
                created_at: "0".into(),
            },
            screenshot_path: None,
            audio_clip_path: None,
            target_word: None,
            definition: None,
            vocab_furigana: None,
            vocab_pitch_num: None,
            sentence_html: None,
        }];

        let count = exporter
            .export_sentences(sentences, "Integration Test".into())
            .await
            .unwrap();
        assert_eq!(count, 1);
    }
}
