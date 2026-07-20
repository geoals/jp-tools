use std::path::PathBuf;

pub struct Config {
    pub db_path: String,
    pub listen_addr: String,
    /// Cached cover images, next to the DB by default.
    pub covers_dir: PathBuf,
    /// Fallback AnkiConnect URL (the dashboard client's IP is probed first).
    pub anki_url: String,
    /// Deck holding mined cards and the field carrying the dictionary form.
    pub anki_deck: String,
    pub anki_vocab_field: String,
    /// Sudachi system dictionary for tokenizing the line stream.
    pub sudachi_dict_path: PathBuf,
    /// vn-mine's capture script, fired by the reader view's mine button.
    pub vn_capture_script: PathBuf,
}

impl Config {
    pub fn from_env() -> Self {
        let db_path = std::env::var("JP_TOOLS_STATS_DB_PATH").unwrap_or_else(|_| {
            let home = std::env::var("HOME").expect("HOME not set");
            format!("{home}/.local/share/jp-stats/stats.db")
        });
        let listen_addr = std::env::var("JP_TOOLS_STATS_LISTEN_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:3200".to_string());
        let covers_dir = std::path::Path::new(&db_path)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("covers");
        Config {
            db_path,
            listen_addr,
            covers_dir,
            anki_url: std::env::var("JP_TOOLS_ANKI_URL")
                .unwrap_or_else(|_| "http://localhost:8765".to_string()),
            anki_deck: std::env::var("JP_TOOLS_ANKI_DECK")
                .unwrap_or_else(|_| "Japanese".to_string()),
            anki_vocab_field: std::env::var("JP_TOOLS_ANKI_FIELD_VOCAB")
                .unwrap_or_else(|_| "VocabKanji".to_string()),
            sudachi_dict_path: std::env::var("JP_TOOLS_SUDACHI_DICT_PATH")
                .unwrap_or_else(|_| "system_full.dic".to_string())
                .into(),
            // Defaults to the sibling crate in this workspace, which is where
            // it lives on the one machine that runs both.
            vn_capture_script: std::env::var("JP_TOOLS_VN_CAPTURE_SH")
                .unwrap_or_else(|_| {
                    concat!(env!("CARGO_MANIFEST_DIR"), "/../vn-mine/vn-capture.sh").to_string()
                })
                .into(),
        }
    }
}
