use std::env;

pub struct Config {
    pub db_path: String,
    pub audio_dir: String,
    pub listen_addr: String,
    pub anki_url: String,
    pub transcribe_script: String,
    /// Number of CPU threads for whisper transcription. 0 = all cores.
    pub whisper_cpu_threads: u32,
    /// Device for whisper transcription: "auto", "cpu", or "cuda".
    pub whisper_device: String,
    /// Directory for temporary media files (screenshots, audio clips).
    pub media_dir: String,
    /// Path to a Yomitan dictionary zip file. Optional — if not set,
    /// VocabDef will be left empty on exported Anki cards.
    pub dictionary_path: Option<String>,
    pub anki: AnkiConfig,
}

/// Anki note type configuration: model name, deck name, and field mapping.
///
/// Each field is `Option<String>` — `Some("FieldName")` means populate that
/// field on the Anki note, `None` means skip it. Defaults match the
/// "Japanese sentences" note type used by Yomitan.
#[derive(Debug, Clone)]
pub struct AnkiConfig {
    pub model_name: String,
    pub deck_name: String,
    pub field_vocab: Option<String>,
    pub field_definition: Option<String>,
    pub field_sentence: Option<String>,
    pub field_image: Option<String>,
    pub field_audio: Option<String>,
    pub field_source: Option<String>,
}

impl Default for AnkiConfig {
    fn default() -> Self {
        Self {
            model_name: "Japanese sentences".into(),
            deck_name: "Japanese".into(),
            field_vocab: Some("VocabKanji".into()),
            field_definition: Some("VocabDef".into()),
            field_sentence: Some("SentKanji".into()),
            field_image: Some("Image".into()),
            field_audio: Some("SentAudio".into()),
            field_source: Some("Document".into()),
        }
    }
}

/// Parse an Anki field mapping env var. Unset = use default, empty = skip field.
fn anki_field(var: &str, default: &str) -> Option<String> {
    match env::var(var) {
        Ok(v) if v.is_empty() => None,
        Ok(v) => Some(v),
        Err(_) => Some(default.into()),
    }
}

impl Config {
    /// Load config from environment variables, falling back to defaults.
    pub fn from_env() -> Self {
        let anki_defaults = AnkiConfig::default();

        Self {
            db_path: env::var("JP_TOOLS_DB_PATH")
                .unwrap_or_else(|_| "jp-tools.db".into()),
            audio_dir: env::var("JP_TOOLS_AUDIO_DIR")
                .unwrap_or_else(|_| "audio".into()),
            listen_addr: env::var("JP_TOOLS_LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:3000".into()),
            anki_url: env::var("JP_TOOLS_ANKI_URL")
                .unwrap_or_else(|_| "http://localhost:8765".into()),
            transcribe_script: env::var("JP_TOOLS_TRANSCRIBE_SCRIPT")
                .unwrap_or_else(|_| "scripts/transcribe.py".into()),
            whisper_cpu_threads: env::var("JP_TOOLS_WHISPER_CPU_THREADS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            whisper_device: env::var("JP_TOOLS_WHISPER_DEVICE")
                .unwrap_or_else(|_| "auto".into()),
            media_dir: env::var("JP_TOOLS_MEDIA_DIR")
                .unwrap_or_else(|_| "media".into()),
            dictionary_path: env::var("JP_TOOLS_DICTIONARY_PATH").ok(),
            anki: AnkiConfig {
                model_name: env::var("JP_TOOLS_ANKI_MODEL")
                    .unwrap_or(anki_defaults.model_name),
                deck_name: env::var("JP_TOOLS_ANKI_DECK")
                    .unwrap_or(anki_defaults.deck_name),
                field_vocab: anki_field(
                    "JP_TOOLS_ANKI_FIELD_VOCAB",
                    anki_defaults.field_vocab.as_deref().unwrap_or(""),
                ),
                field_definition: anki_field(
                    "JP_TOOLS_ANKI_FIELD_DEFINITION",
                    anki_defaults.field_definition.as_deref().unwrap_or(""),
                ),
                field_sentence: anki_field(
                    "JP_TOOLS_ANKI_FIELD_SENTENCE",
                    anki_defaults.field_sentence.as_deref().unwrap_or(""),
                ),
                field_image: anki_field(
                    "JP_TOOLS_ANKI_FIELD_IMAGE",
                    anki_defaults.field_image.as_deref().unwrap_or(""),
                ),
                field_audio: anki_field(
                    "JP_TOOLS_ANKI_FIELD_AUDIO",
                    anki_defaults.field_audio.as_deref().unwrap_or(""),
                ),
                field_source: anki_field(
                    "JP_TOOLS_ANKI_FIELD_SOURCE",
                    anki_defaults.field_source.as_deref().unwrap_or(""),
                ),
            },
        }
    }

    pub fn database_url(&self) -> String {
        format!("sqlite://{}?mode=rwc", self.db_path)
    }
}
