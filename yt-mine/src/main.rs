use std::sync::Arc;

use tracing::info;

use yt_mine::app::{AppState, build_router};
use yt_mine::config::Config;
use yt_mine::db;
use yt_mine::services::download::YtDlpDownloader;
use yt_mine::services::export::AnkiConnectExporter;
use yt_mine::services::media::FfmpegMediaExtractor;
use yt_mine::services::dictionary::Dictionary;
use yt_mine::services::tokenize::LinderaTokenizer;
use yt_mine::services::transcribe::WhisperWorker;

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

    let mut dictionaries: Vec<Arc<Dictionary>> = Vec::new();
    for path in &config.dictionary_paths {
        info!(path, "loading dictionary");
        let dict = Dictionary::load_or_import(&pool, std::path::Path::new(path))
            .await
            .expect("failed to load dictionary");
        dictionaries.push(Arc::new(dict));
    }
    if dictionaries.is_empty() {
        info!("no dictionaries configured (set JP_TOOLS_DICTIONARY_PATHS to enable definitions)");
    }

    let state = AppState {
        db: pool,
        downloader: Arc::new(YtDlpDownloader),
        transcriber: Arc::new(transcriber),
        exporter: Arc::new(AnkiConnectExporter::new(config.anki_url, config.anki)),
        media_extractor: Arc::new(FfmpegMediaExtractor),
        tokenizer: Arc::new(tokenizer),
        dictionaries,
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
