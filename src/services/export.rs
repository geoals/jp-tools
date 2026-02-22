use std::future::Future;
use std::pin::Pin;

use serde_json::{json, Value};

use crate::models::Sentence;

#[cfg_attr(test, mockall::automock)]
pub trait AnkiExporter: Send + Sync {
    fn export_sentences(
        &self,
        sentences: Vec<Sentence>,
        source: String,
    ) -> Pin<Box<dyn Future<Output = Result<usize, ExportError>> + Send>>;
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("export failed: {0}")]
    Failed(String),
}

const MODEL_NAME: &str = "jp-tools-sentence";
const DECK_NAME: &str = "Sentence Mining";

/// Build the AnkiConnect `addNotes` request body.
/// Pure function, easy to test.
pub fn build_add_notes_request(sentences: &[Sentence], source: &str) -> Value {
    let notes: Vec<Value> = sentences
        .iter()
        .map(|s| {
            let timestamp = format_timestamp(s.start_time);
            json!({
                "deckName": DECK_NAME,
                "modelName": MODEL_NAME,
                "fields": {
                    "Sentence": s.text,
                    "Source": format!("{source} ({timestamp})")
                },
                "tags": ["jp-tools", "youtube"]
            })
        })
        .collect();

    json!({
        "action": "addNotes",
        "version": 6,
        "params": {
            "notes": notes
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
            "inOrderFields": ["Sentence", "Source"],
            "cardTemplates": [{
                "Name": "Card 1",
                "Front": "{{Sentence}}",
                "Back": "{{Source}}"
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

pub struct AnkiConnectExporter {
    pub anki_url: String,
    client: reqwest::Client,
}

impl AnkiConnectExporter {
    pub fn new(anki_url: String) -> Self {
        Self {
            anki_url,
            client: reqwest::Client::new(),
        }
    }

}

impl AnkiExporter for AnkiConnectExporter {
    fn export_sentences(
        &self,
        sentences: Vec<Sentence>,
        source: String,
    ) -> Pin<Box<dyn Future<Output = Result<usize, ExportError>> + Send>> {
        let client = self.client.clone();
        let anki_url = self.anki_url.clone();
        let count = sentences.len();

        // Clone what we need for ensure_setup
        let setup_client = self.client.clone();
        let setup_url = self.anki_url.clone();

        Box::pin(async move {
            // Ensure model and deck exist
            // (inline the setup logic since we can't borrow self in the future)
            let _ = setup_client
                .post(&setup_url)
                .json(&build_create_model_request())
                .send()
                .await;

            setup_client
                .post(&setup_url)
                .json(&build_create_deck_request())
                .send()
                .await
                .map_err(|e| ExportError::Failed(format!("failed to create deck: {e}")))?;

            // Add notes
            let request_body = build_add_notes_request(&sentences, &source);

            let response = client
                .post(&anki_url)
                .json(&request_body)
                .send()
                .await
                .map_err(|e| ExportError::Failed(format!("AnkiConnect request failed: {e}")))?;

            let body: Value = response
                .json()
                .await
                .map_err(|e| ExportError::Failed(format!("failed to parse response: {e}")))?;

            if let Some(error) = body.get("error").and_then(|e| e.as_str()) {
                return Err(ExportError::Failed(format!("AnkiConnect error: {error}")));
            }

            Ok(count)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_add_notes_request_structure() {
        let sentences = vec![
            Sentence {
                id: 1,
                job_id: 1,
                text: "テスト文".into(),
                start_time: 5.0,
                end_time: 8.0,
                created_at: "0".into(),
            },
            Sentence {
                id: 2,
                job_id: 1,
                text: "もう一つ".into(),
                start_time: 65.5,
                end_time: 68.0,
                created_at: "0".into(),
            },
        ];

        let request = build_add_notes_request(&sentences, "Test Video");

        assert_eq!(request["action"], "addNotes");
        assert_eq!(request["version"], 6);

        let notes = request["params"]["notes"].as_array().unwrap();
        assert_eq!(notes.len(), 2);

        assert_eq!(notes[0]["deckName"], DECK_NAME);
        assert_eq!(notes[0]["modelName"], MODEL_NAME);
        assert_eq!(notes[0]["fields"]["Sentence"], "テスト文");
        assert_eq!(notes[0]["fields"]["Source"], "Test Video (0:05)");
        assert_eq!(notes[0]["tags"][0], "jp-tools");
        assert_eq!(notes[0]["tags"][1], "youtube");

        // Second note should have formatted timestamp 1:05
        assert_eq!(notes[1]["fields"]["Source"], "Test Video (1:05)");
    }

    #[tokio::test]
    #[ignore = "requires Anki + AnkiConnect running"]
    async fn anki_connect_integration() {
        let exporter = AnkiConnectExporter::new("http://localhost:8765".into());

        let sentences = vec![Sentence {
            id: 1,
            job_id: 1,
            text: "テスト文です".into(),
            start_time: 0.0,
            end_time: 3.0,
            created_at: "0".into(),
        }];

        let count = exporter
            .export_sentences(sentences, "Integration Test".into())
            .await
            .unwrap();
        assert_eq!(count, 1);
    }
}
