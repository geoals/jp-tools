use std::env;
use std::path::PathBuf;

pub use jp_mine_core::config::AnkiConfig;

pub struct Config {
    pub db_path: String,
    /// jp-core's shared knowledge database: the dictionary cache and the
    /// reading record. Separate from this app's own DB.
    pub knowledge_db_path: String,
    pub audio_dir: String,
    pub listen_addr: String,
    pub anki_url: String,
    /// Directory for temporary media files (screenshots, audio clips).
    pub media_dir: String,
    pub anki: AnkiConfig,
    /// When true, use fake implementations of external tools (yt-dlp, whisper,
    /// ffmpeg, AnkiConnect) so the server can run without any dependencies.
    pub fake_api: bool,
    /// Anthropic API key for LLM-generated definitions. When absent, LLM
    /// definitions are skipped entirely.
    pub anthropic_api_key: Option<String>,
    /// Model to use for LLM definitions.
    /// URL of remote whisper-service for transcription.
    pub whisper_service_url: String,
    /// Path to the Sudachi system dictionary (.dic file).
    pub sudachi_dict_path: PathBuf,
}

impl Config {
    /// Load config from environment variables, falling back to defaults.
    pub fn from_env() -> Self {
        Self {
            knowledge_db_path: env::var("JP_TOOLS_KNOWLEDGE_DB_PATH").unwrap_or_else(|_| {
                let home = env::var("HOME").expect("HOME not set");
                format!("{home}/.local/share/jp-tools/knowledge.db")
            }),
            db_path: env::var("JP_TOOLS_DB_PATH").unwrap_or_else(|_| {
                let home = env::var("HOME").expect("HOME not set");
                format!("{home}/.local/share/jp-tools/yt-mine.db")
            }),
            audio_dir: env::var("JP_TOOLS_AUDIO_DIR").unwrap_or_else(|_| "audio".into()),
            listen_addr: env::var("JP_TOOLS_LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".into()),
            anki_url: env::var("JP_TOOLS_ANKI_URL")
                .unwrap_or_else(|_| "http://localhost:8765".into()),
            media_dir: env::var("JP_TOOLS_MEDIA_DIR").unwrap_or_else(|_| "media".into()),
            fake_api: matches!(env::var("JP_TOOLS_FAKE_API").as_deref(), Ok("true" | "1"),),
            anthropic_api_key: env::var("JP_TOOLS_ANTHROPIC_API_KEY").ok(),
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
