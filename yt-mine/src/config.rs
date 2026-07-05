use std::env;
use std::path::PathBuf;

pub use jp_mine_core::config::AnkiConfig;

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
    /// Path to the Sudachi system dictionary (.dic file).
    pub sudachi_dict_path: PathBuf,
}

impl Config {
    /// Load config from environment variables, falling back to defaults.
    pub fn from_env() -> Self {
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
            sudachi_dict_path: env::var("JP_TOOLS_SUDACHI_DICT_PATH")
                .unwrap_or_else(|_| "system_full.dic".into())
                .into(),
            anki: AnkiConfig {
                tags: vec!["yt-mine".into(), "youtube".into()],
                ..AnkiConfig::from_env()
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
