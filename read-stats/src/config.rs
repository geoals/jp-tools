use std::path::PathBuf;

pub struct Config {
    /// read-stats' own database (settings, reader marks, cover sources).
    pub db_path: String,
    /// jp-core's shared knowledge database: the line stream, works, manual
    /// sessions, the Anki mirror, lookups, and the dictionary cache.
    pub knowledge_db_path: String,
    pub listen_addr: String,
    /// Cached cover images, next to the DB by default.
    pub covers_dir: PathBuf,
    /// Fallback AnkiConnect URL (the dashboard client's IP is probed first).
    ///
    /// Numeric, not `localhost`: AnkiConnect binds IPv4 loopback only, and
    /// `localhost` resolves to `::1` first here, so every request failed to
    /// connect while curl's own IPv4 fallback made Anki look reachable.
    pub anki_url: String,
    /// Deck holding mined cards and the field carrying the dictionary form.
    pub anki_deck: String,
    pub anki_vocab_field: String,
    /// Field holding the card's sentence (source text for CompactDef).
    pub anki_sentence_field: String,
    /// Field CompactDef is written to. Empty disables CompactDef enrichment.
    pub anki_compact_def_field: String,
    /// Fire vn-capture.sh (audio + picture) after a card is added. This *is*
    /// mining now — the reader's mine button is gone, because every card added
    /// while reading comes from a line that is on screen, which is exactly when
    /// a capture is wanted. Set to 0 on a machine
    /// that serves the dashboard but doesn't run the VN; where the capture
    /// script is simply absent it already no-ops with a warning.
    pub auto_capture_on_add: bool,
    /// Sudachi system dictionary for tokenizing the line stream.
    pub sudachi_dict_path: PathBuf,
    /// vn-mine's capture script, fired by the reader view's mine button.
    pub vn_capture_script: PathBuf,
    /// Anthropic API key for the reader's "explain this line" button. When
    /// unset the button is disabled rather than the server failing a request.
    pub anthropic_api_key: Option<String>,
    /// whisper-service, used by vn-capture.sh only for the sentence-level trim.
    /// Probed for the reader's status indicator; a capture still works without
    /// it (the clip is attached VAD-trimmed, just not narrowed to one sentence).
    pub whisper_url: String,
}

impl Config {
    pub fn from_env() -> Self {
        let db_path = std::env::var("JP_TOOLS_STATS_DB_PATH").unwrap_or_else(|_| {
            let home = std::env::var("HOME").expect("HOME not set");
            format!("{home}/.local/share/jp-tools/read-stats.db")
        });
        let knowledge_db_path = std::env::var("JP_TOOLS_KNOWLEDGE_DB_PATH").unwrap_or_else(|_| {
            let home = std::env::var("HOME").expect("HOME not set");
            format!("{home}/.local/share/jp-tools/knowledge.db")
        });
        let listen_addr = std::env::var("JP_TOOLS_STATS_LISTEN_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:3200".to_string());
        let covers_dir = std::path::Path::new(&db_path)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("covers");
        Config {
            db_path,
            knowledge_db_path,
            listen_addr,
            covers_dir,
            anki_url: std::env::var("JP_TOOLS_ANKI_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8765".to_string()),
            anki_deck: std::env::var("JP_TOOLS_ANKI_DECK")
                .unwrap_or_else(|_| "Japanese".to_string()),
            anki_vocab_field: std::env::var("JP_TOOLS_ANKI_FIELD_VOCAB")
                .unwrap_or_else(|_| "VocabKanji".to_string()),
            anki_sentence_field: std::env::var("JP_TOOLS_ANKI_FIELD_SENTENCE")
                .unwrap_or_else(|_| "SentKanji".to_string()),
            anki_compact_def_field: std::env::var("JP_TOOLS_ANKI_FIELD_COMPACT_DEF")
                .unwrap_or_else(|_| "CompactDef".to_string()),
            auto_capture_on_add: std::env::var("JP_TOOLS_AUTO_CAPTURE_ON_ADD")
                .map(|v| !(v == "0" || v.eq_ignore_ascii_case("false")))
                .unwrap_or(true),
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
            anthropic_api_key: std::env::var("JP_TOOLS_ANTHROPIC_API_KEY").ok(),
            whisper_url: std::env::var("JP_TOOLS_WHISPER_URL")
                .unwrap_or_else(|_| "http://localhost:8100".to_string()),
        }
    }
}
