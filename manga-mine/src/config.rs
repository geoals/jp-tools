use std::env;
use std::path::PathBuf;

pub use jp_mine_core::config::AnkiConfig;

pub struct Config {
    /// Watched inbox folder — its contents are the mining queue. Photos synced
    /// from the phone land here; mined/skipped photos are moved into
    /// `processed/` / `skipped/` subfolders.
    pub inbox_dir: String,
    pub listen_addr: String,
    pub anki_url: String,
    /// Directory for temporary media files (compressed card images).
    pub media_dir: String,
    /// SQLite database holding the dictionary cache. Shared with yt-mine so
    /// dictionaries imported there are reused (manga-mine stores nothing else).
    pub db_path: String,
    /// Paths to Yomitan dictionary zip files.
    pub dictionary_paths: Vec<String>,
    pub anki: AnkiConfig,
    /// When true, use fake implementations of external tools (OCR service,
    /// AnkiConnect, Sudachi) so the server runs without any dependencies.
    pub fake_api: bool,
    /// URL of the manga-ocr-service.
    pub ocr_service_url: String,
    /// Path to the Sudachi system dictionary (.dic file).
    pub sudachi_dict_path: PathBuf,
    /// When true (default), probe the requesting client's IP for a running
    /// AnkiConnect (port 8765) on export and prefer it over `anki_url` —
    /// mining from the phone then lands cards in the phone's collection.
    pub use_client_anki: bool,
    /// Longest side of the compressed card image in pixels.
    pub card_image_max_dim: u32,
    /// JPEG quality of the compressed card image.
    pub card_image_quality: u8,
}

impl Config {
    /// Load config from environment variables, falling back to defaults.
    pub fn from_env() -> Self {
        Self {
            inbox_dir: env::var("JP_TOOLS_MANGA_INBOX").unwrap_or_else(|_| "manga-inbox".into()),
            listen_addr: env::var("JP_TOOLS_MANGA_LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:3100".into()),
            anki_url: env::var("JP_TOOLS_ANKI_URL")
                .unwrap_or_else(|_| "http://localhost:8765".into()),
            media_dir: env::var("JP_TOOLS_MEDIA_DIR").unwrap_or_else(|_| "media".into()),
            db_path: env::var("JP_TOOLS_DB_PATH").unwrap_or_else(|_| "yt-mine.db".into()),
            dictionary_paths: parse_dictionary_paths(),
            fake_api: matches!(env::var("JP_TOOLS_FAKE_API").as_deref(), Ok("true" | "1"),),
            ocr_service_url: env::var("JP_TOOLS_OCR_SERVICE_URL")
                .unwrap_or_else(|_| "http://localhost:8200".into()),
            sudachi_dict_path: env::var("JP_TOOLS_SUDACHI_DICT_PATH")
                .unwrap_or_else(|_| "system_full.dic".into())
                .into(),
            use_client_anki: !matches!(
                env::var("JP_TOOLS_ANKI_USE_CLIENT").as_deref(),
                Ok("false" | "0"),
            ),
            card_image_max_dim: env::var("JP_TOOLS_MANGA_CARD_IMAGE_MAX_DIM")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1280),
            card_image_quality: env::var("JP_TOOLS_MANGA_CARD_IMAGE_QUALITY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(80),
            anki: AnkiConfig {
                tags: vec!["manga-mine".into(), "manga".into()],
                ..AnkiConfig::from_env()
            },
        }
    }

    pub fn database_url(&self) -> String {
        format!("sqlite://{}?mode=rwc", self.db_path)
    }
}

/// Parse dictionary paths from environment (comma-separated).
fn parse_dictionary_paths() -> Vec<String> {
    if let Ok(paths) = env::var("JP_TOOLS_DICTIONARY_PATHS") {
        return paths
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    env::var("JP_TOOLS_DICTIONARY_PATH")
        .ok()
        .into_iter()
        .collect()
}
