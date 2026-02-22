use std::sync::Arc;

use tracing::info;

use jp_tools::app::{AppState, build_router};
use jp_tools::config::Config;
use jp_tools::db;
use jp_tools::services::download::YtDlpDownloader;
use jp_tools::services::export::AnkiConnectExporter;
use jp_tools::services::media::FfmpegMediaExtractor;
use jp_tools::services::dictionary::Dictionary;
use jp_tools::services::tokenize::LinderaTokenizer;
use jp_tools::services::transcribe::WhisperWorker;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let config = Config::from_env();

    // Ensure output directories exist
    std::fs::create_dir_all(&config.audio_dir).expect("failed to create audio directory");
    std::fs::create_dir_all(&config.media_dir).expect("failed to create media directory");
    let media_dir = std::fs::canonicalize(&config.media_dir)
        .expect("failed to resolve media directory")
        .to_string_lossy()
        .into_owned();

    let pool = db::create_pool(&config.database_url())
        .await
        .expect("failed to create database pool");

    let transcriber = WhisperWorker::spawn(
        &config.transcribe_script,
        config.whisper_cpu_threads,
        &config.whisper_device,
    )
    .await
    .expect("failed to start whisper worker");

    let tokenizer = LinderaTokenizer::new().expect("failed to initialize tokenizer");

    let dictionary = config.dictionary_path.as_ref().map(|path| {
        info!(path, "loading dictionary");
        Arc::new(
            Dictionary::load_from_zip(std::path::Path::new(path))
                .expect("failed to load dictionary"),
        )
    });
    if dictionary.is_none() {
        info!("no dictionary configured (set JP_TOOLS_DICTIONARY_PATH to enable definitions)");
    }

    let state = AppState {
        db: pool,
        downloader: Arc::new(YtDlpDownloader),
        transcriber: Arc::new(transcriber),
        exporter: Arc::new(AnkiConnectExporter::new(config.anki_url, config.anki)),
        media_extractor: Arc::new(FfmpegMediaExtractor),
        tokenizer: Arc::new(tokenizer),
        dictionary,
        audio_dir: config.audio_dir,
        media_dir,
    };

    let router = build_router(state);

    let listener = tokio::net::TcpListener::bind(&config.listen_addr)
        .await
        .expect("failed to bind listener");

    info!(addr = %config.listen_addr, "server starting");
    axum::serve(listener, router).await.expect("server error");
}
