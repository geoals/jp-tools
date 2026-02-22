use std::future::Future;
use std::pin::Pin;

use serde_json::{json, Value};
use tokio::fs;
use tracing::{debug, warn};

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

const MODEL_NAME: &str = "jp-tools-mining";
const DECK_NAME: &str = "Sentence Mining";

/// Data for a single Anki note, ready to be serialized.
pub struct NoteData {
    pub sentence_text: String,
    pub vocab_kanji: String,
    pub vocab_def: String,
    pub source: String,
    pub screenshot_filename: Option<String>,
    pub audio_clip_filename: Option<String>,
}

/// Build the AnkiConnect `addNotes` request body.
pub fn build_add_notes_request(notes: &[NoteData]) -> Value {
    let notes_json: Vec<Value> = notes
        .iter()
        .map(|n| {
            let image_field = n
                .screenshot_filename
                .as_deref()
                .map(|f| format!("<img src=\"{f}\">"))
                .unwrap_or_default();

            let audio_field = n
                .audio_clip_filename
                .as_deref()
                .map(|f| format!("[sound:{f}]"))
                .unwrap_or_default();

            json!({
                "deckName": DECK_NAME,
                "modelName": MODEL_NAME,
                "fields": {
                    "VocabKanji": n.vocab_kanji,
                    "VocabDef": n.vocab_def,
                    "SentKanji": n.sentence_text,
                    "Image": image_field,
                    "SentAudio": audio_field,
                    "Document": n.source,
                },
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

/// Build the request to create the note type if it doesn't exist.
fn build_create_model_request() -> Value {
    json!({
        "action": "createModel",
        "version": 6,
        "params": {
            "modelName": MODEL_NAME,
            "inOrderFields": ["VocabKanji", "VocabDef", "SentKanji", "Image", "SentAudio", "Document"],
            "cardTemplates": [{
                "Name": "Card 1",
                "Front": "{{SentKanji}}",
                "Back": "{{VocabKanji}}<br>{{VocabDef}}<br>{{Image}}<br>{{SentAudio}}<br>{{Document}}"
            }]
        }
    })
}

/// Build the request to create the deck if it doesn't exist.
fn build_create_deck_request() -> Value {
    json!({
        "action": "createDeck",
        "version": 6,
        "params": {
            "deck": DECK_NAME
        }
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
    client: reqwest::Client,
}

impl AnkiConnectExporter {
    pub fn new(anki_url: String) -> Self {
        Self {
            anki_url,
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

impl AnkiExporter for AnkiConnectExporter {
    fn export_sentences(
        &self,
        sentences: Vec<ExportSentence>,
        source: String,
    ) -> Pin<Box<dyn Future<Output = Result<usize, ExportError>> + Send>> {
        let client = self.client.clone();
        let anki_url = self.anki_url.clone();
        let count = sentences.len();

        Box::pin(async move {
            // Ensure model and deck exist (ignore "already exists" errors on model)
            let _ = send_anki_request(
                &client,
                &anki_url,
                &build_create_model_request(),
            )
            .await;

            send_anki_request(&client, &anki_url, &build_create_deck_request()).await?;

            // Upload media files (read bytes and send as base64)
            for es in &sentences {
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
                        sentence_text: es.sentence.text.clone(),
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
                    }
                })
                .collect();

            // Add notes
            let add_notes_req = build_add_notes_request(&note_data);
            debug!(request = %add_notes_req, "sending addNotes to AnkiConnect");
            send_anki_request(&client, &anki_url, &add_notes_req).await?;

            Ok(count)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_add_notes_request_structure() {
        let notes = vec![
            NoteData {
                sentence_text: "テスト文".into(),
                vocab_kanji: "テスト".into(),
                vocab_def: "test".into(),
                source: "Test Video (0:05)".into(),
                screenshot_filename: Some("jp-tools_1_1.jpg".into()),
                audio_clip_filename: Some("jp-tools_1_1.mp3".into()),
            },
            NoteData {
                sentence_text: "もう一つ".into(),
                vocab_kanji: "もう一つ".into(),
                vocab_def: "".into(),
                source: "Test Video (1:05)".into(),
                screenshot_filename: None,
                audio_clip_filename: None,
            },
        ];

        let request = build_add_notes_request(&notes);

        assert_eq!(request["action"], "addNotes");
        assert_eq!(request["version"], 6);

        let result_notes = request["params"]["notes"].as_array().unwrap();
        assert_eq!(result_notes.len(), 2);

        assert_eq!(result_notes[0]["deckName"], DECK_NAME);
        assert_eq!(result_notes[0]["modelName"], MODEL_NAME);
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

        // Second note: no media, no target word
        assert_eq!(result_notes[1]["fields"]["Image"], "");
        assert_eq!(result_notes[1]["fields"]["SentAudio"], "");
        assert_eq!(result_notes[1]["fields"]["VocabKanji"], "もう一つ");
        assert_eq!(result_notes[1]["fields"]["VocabDef"], "");
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
        let exporter = AnkiConnectExporter::new("http://localhost:8765".into());

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
        }];

        let count = exporter
            .export_sentences(sentences, "Integration Test".into())
            .await
            .unwrap();
        assert_eq!(count, 1);
    }
}
