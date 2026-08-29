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
    pub db_path: String,
    /// jp-core's shared knowledge database: the dictionary cache and the
    /// reading record. Separate from this app's own DB.
    pub knowledge_db_path: String,
    pub anki: AnkiConfig,
    /// Swap every external tool — the OCR service, Anki, Sudachi — for a fake, so
    /// the server runs with none of them installed.
    pub fake_api: bool,
    pub ocr_service_url: String,
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
    pub fn from_env() -> Self {
        Self {
            inbox_dir: env::var("KOTODEX_MANGA_INBOX").unwrap_or_else(|_| "manga-inbox".into()),
            listen_addr: env::var("KOTODEX_MANGA_LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:3100".into()),
            anki_url: env::var("KOTODEX_ANKI_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8765".into()),
            media_dir: env::var("KOTODEX_MEDIA_DIR").unwrap_or_else(|_| "media".into()),
            knowledge_db_path: env::var("KOTODEX_KNOWLEDGE_DB_PATH").unwrap_or_else(|_| {
                let home = env::var("HOME").expect("HOME not set");
                format!("{home}/.local/share/kotodex/knowledge.db")
            }),
            db_path: env::var("KOTODEX_DB_PATH").unwrap_or_else(|_| {
                let home = env::var("HOME").expect("HOME not set");
                format!("{home}/.local/share/kotodex/yt-mine.db")
            }),
            fake_api: matches!(env::var("KOTODEX_FAKE_API").as_deref(), Ok("true" | "1"),),
            ocr_service_url: env::var("KOTODEX_OCR_SERVICE_URL")
                .unwrap_or_else(|_| "http://localhost:8200".into()),
            sudachi_dict_path: env::var("KOTODEX_SUDACHI_DICT_PATH")
                .unwrap_or_else(|_| "system_full.dic".into())
                .into(),
            use_client_anki: !matches!(
                env::var("KOTODEX_ANKI_USE_CLIENT").as_deref(),
                Ok("false" | "0"),
            ),
            card_image_max_dim: env::var("KOTODEX_MANGA_CARD_IMAGE_MAX_DIM")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1280),
            card_image_quality: env::var("KOTODEX_MANGA_CARD_IMAGE_QUALITY")
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
