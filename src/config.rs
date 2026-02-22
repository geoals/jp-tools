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
}

impl Config {
    /// Load config from environment variables, falling back to defaults.
    pub fn from_env() -> Self {
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
        }
    }

    pub fn database_url(&self) -> String {
        format!("sqlite://{}?mode=rwc", self.db_path)
    }
}
