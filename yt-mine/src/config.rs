use std::env;

pub struct Config {
    pub db_path: String,
    pub audio_dir: String,
    pub listen_addr: String,
    pub anki_url: String,
    /// Directory for temporary media files (screenshots, audio clips).
    pub media_dir: String,
    /// Paths to Yomitan dictionary zip files. If empty, VocabDef will be
    /// left empty on exported Anki cards.
    pub dictionary_paths: Vec<String>,
    pub anki: AnkiConfig,
    /// When true, use fake implementations of external tools (yt-dlp, whisper,
    /// ffmpeg, AnkiConnect) so the server can run without any dependencies.
    pub fake_api: bool,
    /// Anthropic API key for LLM-generated definitions. When absent, LLM
    /// definitions are skipped entirely.
    pub anthropic_api_key: Option<String>,
    /// Model to use for LLM definitions.
    pub llm_model: String,
    /// URL of remote whisper-service for transcription.
    pub whisper_service_url: String,
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
    pub field_furigana: Option<String>,
    pub field_pitch_num: Option<String>,
    pub field_llm_definition: Option<String>,
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
            field_furigana: Some("VocabFurigana".into()),
            field_pitch_num: Some("VocabPitchNum".into()),
            field_llm_definition: Some("LLMDef".into()),
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
                .unwrap_or_else(|_| "yt-mine.db".into()),
            audio_dir: env::var("JP_TOOLS_AUDIO_DIR")
                .unwrap_or_else(|_| "audio".into()),
            listen_addr: env::var("JP_TOOLS_LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:3000".into()),
            anki_url: env::var("JP_TOOLS_ANKI_URL")
                .unwrap_or_else(|_| "http://localhost:8765".into()),
            media_dir: env::var("JP_TOOLS_MEDIA_DIR")
                .unwrap_or_else(|_| "media".into()),
            dictionary_paths: parse_dictionary_paths(),
            fake_api: matches!(
                env::var("JP_TOOLS_FAKE_API").as_deref(),
                Ok("true" | "1"),
            ),
            anthropic_api_key: env::var("JP_TOOLS_ANTHROPIC_API_KEY").ok(),
            llm_model: env::var("JP_TOOLS_LLM_MODEL")
                .unwrap_or_else(|_| "claude-haiku-4-5".into()),
            whisper_service_url: env::var("JP_TOOLS_WHISPER_SERVICE_URL")
                .unwrap_or_else(|_| "http://localhost:8100".into()),
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
                field_furigana: anki_field(
                    "JP_TOOLS_ANKI_FIELD_FURIGANA",
                    anki_defaults.field_furigana.as_deref().unwrap_or(""),
                ),
                field_pitch_num: anki_field(
                    "JP_TOOLS_ANKI_FIELD_PITCH_NUM",
                    anki_defaults.field_pitch_num.as_deref().unwrap_or(""),
                ),
                field_llm_definition: anki_field(
                    "JP_TOOLS_ANKI_FIELD_LLM_DEFINITION",
                    anki_defaults.field_llm_definition.as_deref().unwrap_or(""),
                ),
            },
        }
    }

    pub fn database_url(&self) -> String {
        format!("sqlite://{}?mode=rwc", self.db_path)
    }
}

/// Parse dictionary paths from environment.
/// Supports `JP_TOOLS_DICTIONARY_PATHS` (comma-separated) with fallback
/// to `JP_TOOLS_DICTIONARY_PATH` (single path) for backward compatibility.
fn parse_dictionary_paths() -> Vec<String> {
    if let Ok(paths) = env::var("JP_TOOLS_DICTIONARY_PATHS") {
        return paths
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    // Backward compat: single path
    env::var("JP_TOOLS_DICTIONARY_PATH")
        .ok()
        .into_iter()
        .collect()
}
