use std::future::Future;
use std::pin::Pin;

use serde_json::{Map, Value, json};
use tokio::fs;
use tracing::{debug, info, warn};

use crate::config::AnkiConfig;

/// A sentence bundled with optional media file paths for Anki export.
///
/// Domain-neutral: callers provide the plain sentence text and a fully
/// formatted per-note source string (e.g. "Video Title (1:05)" or a manga
/// photo name).
#[derive(Debug, Clone)]
pub struct ExportSentence {
    /// Plain sentence text (fallback when `sentence_html` is absent).
    pub sentence_text: String,
    /// Fully formatted source string for this note.
    pub source: String,
    /// Absolute path to the image jpg on disk (if available).
    pub screenshot_path: Option<String>,
    /// Absolute path to the audio clip mp3 on disk (if available).
    pub audio_clip_path: Option<String>,
    /// The target vocabulary word selected by the user (base/dictionary form).
    pub target_word: Option<String>,
    /// Dictionary definition of the target word, if found.
    pub definition: Option<String>,
    /// Anki bracket furigana notation, e.g. `隔週[かくしゅう]`.
    pub vocab_furigana: Option<String>,
    /// Pitch accent downstep position(s), e.g. `"0"` or `"0,3"`.
    pub vocab_pitch_num: Option<String>,
    /// Frequency rank of the target word (lower = more common), e.g. 2000.
    pub vocab_frequency: Option<i64>,
    /// Sentence text with the target word wrapped in `<b></b>` for Anki display.
    pub sentence_html: Option<String>,
    /// LLM-generated definition/explanation of the target word.
    pub llm_definition: Option<String>,
}

#[cfg_attr(any(test, feature = "test-support"), mockall::automock)]
pub trait AnkiExporter: Send + Sync {
    fn export_sentences(
        &self,
        sentences: Vec<ExportSentence>,
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
    /// Frequency rank as a plain number string, empty when unknown.
    pub vocab_frequency: String,
    pub llm_definition: String,
}

/// Build one AnkiConnect `addNote` request using the configured field mapping.
/// Only fields that are mapped (Some) in the config are included on the note.
///
/// `addNote` (singular) is used instead of `addNotes` because subset
/// implementations like AnkiconnectAndroid only support the former; the full
/// Yomitan-style `options` object is included because AnkiconnectAndroid's
/// parser requires every key of it.
pub fn build_add_note_request(n: &NoteData, config: &AnkiConfig) -> Value {
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
    if let Some(ref f) = config.field_frequency {
        fields.insert(f.clone(), json!(n.vocab_frequency));
    }
    if let Some(ref f) = config.field_llm_definition {
        fields.insert(f.clone(), json!(n.llm_definition));
    }

    json!({
        "action": "addNote",
        "version": 6,
        "params": {
            "note": {
                "deckName": config.deck_name,
                "modelName": config.model_name,
                "fields": fields,
                "tags": config.tags,
                "options": {
                    "allowDuplicate": false,
                    "duplicateScope": "collection",
                    "duplicateScopeOptions": {
                        "deckName": null,
                        "checkChildren": false,
                        "checkAllModels": false
                    }
                }
            }
        }
    })
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
        config.field_frequency.as_deref(),
        config.field_llm_definition.as_deref(),
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
/// AnkiConnect runs on a different machine than the mining server.
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

/// Upload one media file via `storeMediaFile` and return the filename it was
/// actually stored under (AnkiConnect may rename). Returns `None` (and warns)
/// when the local file can't be read — the note is then exported without it.
async fn upload_media(
    client: &reqwest::Client,
    anki_url: &str,
    path: &str,
    fallback_name: &str,
) -> Result<Option<String>, ExportError> {
    let requested = std::path::Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or(fallback_name);

    let data = match read_file_base64(path).await {
        Ok(data) => data,
        Err(e) => {
            warn!(path, error = %e, "skipping media upload");
            return Ok(None);
        }
    };

    let req = build_store_media_request(requested, &data);
    let resp = send_anki_request(client, anki_url, &req).await?;
    Ok(Some(
        resp["result"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| requested.to_owned()),
    ))
}

pub struct AnkiConnectExporter {
    pub anki_url: String,
    pub config: AnkiConfig,
    client: reqwest::Client,
    /// Whether model/deck setup has already succeeded against this target —
    /// it only needs to happen once per Anki instance, and skipping it saves
    /// two round-trips per export (which dominate on slow LAN targets).
    setup_done: std::sync::Arc<std::sync::atomic::AtomicBool>,
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
            setup_done: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// Send a request to AnkiConnect and check for errors in the response.
async fn send_anki_request(
    client: &reqwest::Client,
    url: &str,
    body: &Value,
) -> Result<Value, ExportError> {
    let action = body["action"].as_str().unwrap_or("?").to_owned();
    let started = std::time::Instant::now();
    let response = client.post(url).json(body).send().await.map_err(|e| {
        ExportError::Failed(format!("AnkiConnect request '{action}' failed: {e:?}"))
    })?;

    let body: Value = response
        .json()
        .await
        .map_err(|e| ExportError::Failed(format!("failed to parse '{action}' response: {e}")))?;
    debug!(
        action,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "anki request"
    );

    // AnkiConnect may return errors as a string or as a non-null JSON value
    match body.get("error") {
        Some(Value::String(error)) => {
            return Err(ExportError::Failed(format!(
                "AnkiConnect error on '{action}': {error}"
            )));
        }
        Some(Value::Null) | None => {}
        Some(other) => {
            return Err(ExportError::Failed(format!(
                "AnkiConnect error on '{action}': {other}"
            )));
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
    ) -> Pin<Box<dyn Future<Output = Result<usize, ExportError>> + Send>> {
        let client = self.client.clone();
        let anki_url = self.anki_url.clone();
        let config = self.config.clone();
        let count = sentences.len();
        let setup_done = std::sync::Arc::clone(&self.setup_done);

        Box::pin(async move {
            let export_started = std::time::Instant::now();
            let setup_started = std::time::Instant::now();
            // Setup (model/deck creation) runs once per target and is
            // best-effort: subset AnkiConnect implementations
            // (AnkiconnectAndroid) don't support createModel/createDeck, and a
            // synced collection already has both. Real problems still surface
            // on addNote below.
            if !setup_done.load(std::sync::atomic::Ordering::Relaxed) {
                match model_exists(&client, &anki_url, &config.model_name).await {
                    Ok(true) => {}
                    Ok(false) => {
                        info!(model = %config.model_name, "model not found, creating fallback");
                        if let Err(e) = send_anki_request(
                            &client,
                            &anki_url,
                            &build_create_model_request(&config),
                        )
                        .await
                        {
                            warn!(error = %e, "createModel failed, continuing");
                        }
                    }
                    Err(e) => warn!(error = %e, "could not check models, continuing"),
                }

                if let Err(e) = send_anki_request(
                    &client,
                    &anki_url,
                    &build_create_deck_request(&config.deck_name),
                )
                .await
                {
                    warn!(error = %e, "createDeck failed, continuing (deck may already exist)");
                }
                setup_done.store(true, std::sync::atomic::Ordering::Relaxed);
            }

            let setup_ms = setup_started.elapsed().as_millis() as u64;

            // Per sentence: upload media, then add the note referencing the
            // *actual* stored filenames — AnkiConnect may rename on store
            // (AnkiconnectAndroid always does), and the response carries the
            // name that ended up in the media folder. addNote (singular) is
            // the lowest common denominator across AnkiConnect implementations.
            let mut media_ms: u64 = 0;
            let mut notes_ms: u64 = 0;
            for es in &sentences {
                let media_started = std::time::Instant::now();
                let mut screenshot_filename = None;
                if config.field_image.is_some() {
                    if let Some(path) = &es.screenshot_path {
                        screenshot_filename =
                            upload_media(&client, &anki_url, path, "screenshot.jpg").await?;
                    }
                }

                let mut audio_clip_filename = None;
                if config.field_audio.is_some() {
                    if let Some(path) = &es.audio_clip_path {
                        audio_clip_filename =
                            upload_media(&client, &anki_url, path, "clip.mp3").await?;
                    }
                }
                media_ms += media_started.elapsed().as_millis() as u64;
                let note_started = std::time::Instant::now();

                let note = NoteData {
                    sentence_text: es
                        .sentence_html
                        .clone()
                        .unwrap_or_else(|| es.sentence_text.clone()),
                    vocab_kanji: es
                        .target_word
                        .clone()
                        .unwrap_or_else(|| es.sentence_text.clone()),
                    vocab_def: es.definition.clone().unwrap_or_default(),
                    source: es.source.clone(),
                    screenshot_filename,
                    audio_clip_filename,
                    vocab_furigana: es.vocab_furigana.clone().unwrap_or_default(),
                    vocab_pitch_num: es.vocab_pitch_num.clone().unwrap_or_default(),
                    vocab_frequency: es
                        .vocab_frequency
                        .map(|f| f.to_string())
                        .unwrap_or_default(),
                    llm_definition: es.llm_definition.clone().unwrap_or_default(),
                };

                let add_note_req = build_add_note_request(&note, &config);
                debug!(request = %add_note_req, "sending addNote to AnkiConnect");
                send_anki_request(&client, &anki_url, &add_note_req).await?;
                notes_ms += note_started.elapsed().as_millis() as u64;
            }

            info!(
                setup_ms,
                media_ms,
                notes_ms,
                total_ms = export_started.elapsed().as_millis() as u64,
                count,
                "anki export timing"
            );
            Ok(count)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> AnkiConfig {
        AnkiConfig {
            tags: vec!["yt-mine".into(), "youtube".into()],
            ..AnkiConfig::default()
        }
    }

    #[test]
    fn build_add_note_request_structure() {
        let config = default_config();
        let note = NoteData {
            sentence_text: "テスト文".into(),
            vocab_kanji: "テスト".into(),
            vocab_def: "test".into(),
            source: "Test Video (0:05)".into(),
            screenshot_filename: Some("yt-mine_1_1.jpg".into()),
            audio_clip_filename: Some("yt-mine_1_1.mp3".into()),
            vocab_furigana: "テスト".into(),
            vocab_pitch_num: "0".into(),
            vocab_frequency: "2000".into(),
            llm_definition: "".into(),
        };

        let request = build_add_note_request(&note, &config);

        assert_eq!(request["action"], "addNote");
        assert_eq!(request["version"], 6);

        let result_note = &request["params"]["note"];
        assert_eq!(result_note["deckName"], "Japanese");
        assert_eq!(result_note["modelName"], "Japanese sentences");
        assert_eq!(result_note["tags"], json!(["yt-mine", "youtube"]));
        assert_eq!(result_note["fields"]["SentKanji"], "テスト文");
        assert_eq!(
            result_note["fields"]["Image"],
            "<img src=\"yt-mine_1_1.jpg\">"
        );
        assert_eq!(
            result_note["fields"]["SentAudio"],
            "[sound:yt-mine_1_1.mp3]"
        );
        assert_eq!(result_note["fields"]["Document"], "Test Video (0:05)");
        assert_eq!(result_note["fields"]["VocabKanji"], "テスト");
        assert_eq!(result_note["fields"]["VocabDef"], "test");
        assert_eq!(result_note["fields"]["VocabFurigana"], "テスト");
        assert_eq!(result_note["fields"]["VocabPitchNum"], "0");
        assert_eq!(result_note["fields"]["Frequency"], "2000");

        // AnkiconnectAndroid requires the full options object
        let options = &result_note["options"];
        assert_eq!(options["allowDuplicate"], false);
        assert_eq!(options["duplicateScope"], "collection");
        assert_eq!(options["duplicateScopeOptions"]["checkChildren"], false);
        assert_eq!(options["duplicateScopeOptions"]["checkAllModels"], false);
        assert!(options["duplicateScopeOptions"]["deckName"].is_null());
    }

    #[test]
    fn build_add_note_request_empty_media() {
        let config = default_config();
        let note = NoteData {
            sentence_text: "もう一つ".into(),
            vocab_kanji: "もう一つ".into(),
            vocab_def: "".into(),
            source: "Test Video (1:05)".into(),
            screenshot_filename: None,
            audio_clip_filename: None,
            vocab_furigana: "".into(),
            vocab_pitch_num: "".into(),
            vocab_frequency: "".into(),
            llm_definition: "".into(),
        };

        let request = build_add_note_request(&note, &config);
        let fields = &request["params"]["note"]["fields"];

        assert_eq!(fields["Image"], "");
        assert_eq!(fields["SentAudio"], "");
        assert_eq!(fields["VocabKanji"], "もう一つ");
        assert_eq!(fields["VocabDef"], "");
        assert_eq!(fields["VocabFurigana"], "");
        assert_eq!(fields["VocabPitchNum"], "");
        assert_eq!(fields["Frequency"], "");
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
            field_frequency: None,
            field_llm_definition: None,
            tags: vec![],
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
            vocab_frequency: "500".into(),
            llm_definition: "llm def".into(),
        }];

        let request = build_add_note_request(&notes[0], &config);
        let fields = &request["params"]["note"]["fields"];

        assert_eq!(fields["Word"], "テスト");
        assert_eq!(fields["Sentence"], "テスト");
        assert!(fields.get("VocabDef").is_none());
        assert!(fields.get("Image").is_none());
        assert!(fields.get("SentAudio").is_none());
        assert!(fields.get("Document").is_none());
        assert!(fields.get("VocabFurigana").is_none());
        assert!(fields.get("VocabPitchNum").is_none());
        assert!(fields.get("Frequency").is_none());
        assert!(fields.get("LLMDef").is_none());
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
            field_frequency: Some("FreqRank".into()),
            field_llm_definition: Some("AIDef".into()),
            tags: vec![],
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
            vocab_frequency: "1234".into(),
            llm_definition: "ai definition".into(),
        }];

        let request = build_add_note_request(&notes[0], &config);
        let note = &request["params"]["note"];

        assert_eq!(note["modelName"], "Custom");
        assert_eq!(note["deckName"], "MyDeck");
        assert_eq!(note["fields"]["Expression"], "語");
        assert_eq!(note["fields"]["Meaning"], "def");
        assert_eq!(note["fields"]["Context"], "文");
        assert_eq!(note["fields"]["Furigana"], "語[ご]");
        assert_eq!(note["fields"]["PitchNum"], "1");
        assert_eq!(note["fields"]["FreqRank"], "1234");
        assert_eq!(note["fields"]["AIDef"], "ai definition");
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
            sentence_text: "テスト文です".into(),
            source: "Integration Test".into(),
            screenshot_path: None,
            audio_clip_path: None,
            target_word: None,
            definition: None,
            vocab_furigana: None,
            vocab_pitch_num: None,
            vocab_frequency: None,
            sentence_html: None,
            llm_definition: None,
        }];

        let count = exporter.export_sentences(sentences).await.unwrap();
        assert_eq!(count, 1);
    }
}
