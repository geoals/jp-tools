use std::path::PathBuf;

pub struct Config {
    /// kotodex-server's own database (settings, reader marks, cover sources).
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
    /// `localhost` resolves to `::1` first here.
    pub anki_url: String,
    /// The note type and its field names, from `AnkiConfig` so that every card
    /// path spells them the same way. The three below are the ones enough code
    /// reads to be worth naming; the rest are reached through `anki`.
    pub anki: jp_mine_core::config::AnkiConfig,
    /// Deck holding mined cards and the field carrying the dictionary form.
    pub anki_deck: String,
    pub anki_vocab_field: String,
    /// Field holding the card's sentence (source text for CompactDef).
    pub anki_sentence_field: String,
    /// Field CompactDef is written to. Empty disables CompactDef enrichment.
    pub anki_compact_def_field: String,
    /// Fire vn-capture.sh (audio + picture) after a card is added. Every card
    /// added while reading comes from a line that is on screen, which is when a
    /// capture is wanted, so this is what makes an add a mine. Set to 0 on a
    /// machine that serves the dashboard but doesn't run the VN; an absent
    /// capture script no-ops with a warning.
    pub auto_capture_on_add: bool,
    /// Sudachi system dictionary for tokenizing the line stream.
    pub sudachi_dict_path: PathBuf,
    /// `capture/vn-capture.sh`, fired by the reader's mine button.
    pub vn_capture_script: PathBuf,
    /// The fallback behind the key stored in `settings`, which is where the
    /// reader sets one. `setup.sh` writes this one.
    pub anthropic_api_key: Option<String>,
    /// whisper-service, used by vn-capture.sh only for the sentence-level trim.
    /// Probed for the reader's status indicator; a capture still works without
    /// it (the clip is attached VAD-trimmed, just not narrowed to one sentence).
    pub whisper_url: String,
    /// The Local Audio Server for Yomitan, an Anki add-on: the popup's 🔊.
    /// Down or absent means no audio button, never a failed popup.
    pub local_audio_url: String,
    /// Serve a frozen copy of someone else's reading, and change nothing.
    ///
    /// The public demo. Every request that is not a GET is refused, and the
    /// boot-time writes are skipped, so a shared instance stays exactly as its
    /// seed left it however many people click through it.
    pub demo: bool,
}

impl Config {
    pub fn from_env() -> Self {
        let db_path = std::env::var("KOTODEX_SERVER_DB_PATH").unwrap_or_else(|_| {
            jp_core::install::data_dir()
                .join("kotodex.db")
                .display()
                .to_string()
        });
        let knowledge_db_path = std::env::var("KOTODEX_KNOWLEDGE_DB_PATH").unwrap_or_else(|_| {
            jp_core::install::data_dir()
                .join("knowledge.db")
                .display()
                .to_string()
        });
        let listen_addr = std::env::var("KOTODEX_SERVER_LISTEN_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:3200".to_string());
        // One field map for every card path. Reading the same env vars again here
        // with defaults of their own is how this crate and the exporter come to
        // disagree about a field name.
        let anki = jp_mine_core::config::AnkiConfig::from_env();
        fn field(name: &Option<String>) -> String {
            name.clone().unwrap_or_default()
        }

        let covers_dir = std::path::Path::new(&db_path)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("covers");
        Config {
            db_path,
            knowledge_db_path,
            listen_addr,
            covers_dir,
            anki_url: std::env::var("KOTODEX_ANKI_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8765".to_string()),
            anki_deck: anki.deck_name.clone(),
            anki_vocab_field: field(&anki.field_vocab),
            anki_sentence_field: field(&anki.field_sentence),
            anki_compact_def_field: field(&anki.field_compact_def),
            anki,
            auto_capture_on_add: std::env::var("KOTODEX_AUTO_CAPTURE_ON_ADD")
                .map(|v| !(v == "0" || v.eq_ignore_ascii_case("false")))
                .unwrap_or(true),
            sudachi_dict_path: std::env::var("KOTODEX_SUDACHI_DICT_PATH")
                .map(Into::into)
                .unwrap_or_else(|_| jp_core::install::install_root().join("system_full.dic")),
            vn_capture_script: std::env::var("KOTODEX_VN_CAPTURE_SH")
                .map(Into::into)
                .unwrap_or_else(|_| jp_core::install::install_root().join("capture/vn-capture.sh")),
            anthropic_api_key: std::env::var("KOTODEX_ANTHROPIC_API_KEY").ok(),
            whisper_url: std::env::var("KOTODEX_WHISPER_URL")
                .unwrap_or_else(|_| "http://localhost:8100".to_string()),
            // Numeric for the same reason as `anki_url`: the add-on binds IPv4
            // loopback and `localhost` resolves to `::1` first here.
            local_audio_url: std::env::var("KOTODEX_LOCAL_AUDIO_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:5050".to_string()),
            demo: std::env::var("KOTODEX_DEMO")
                .map(|v| !(v.is_empty() || v == "0" || v.eq_ignore_ascii_case("false")))
                .unwrap_or(false),
        }
    }
}
