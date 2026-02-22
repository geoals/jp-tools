mod app;
mod config;
mod db;
mod error;
mod models;
mod routes;
mod services;

use std::sync::Arc;

use tracing::info;

use crate::app::{AppState, build_router};
use crate::config::Config;
use crate::services::download::YtDlpDownloader;
use crate::services::export::AnkiConnectExporter;
use crate::services::media::FfmpegMediaExtractor;
use crate::services::transcribe::WhisperWorker;

#[tokio::main]
async fn main() {
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

    let state = AppState {
        db: pool,
        downloader: Arc::new(YtDlpDownloader),
        transcriber: Arc::new(transcriber),
        exporter: Arc::new(AnkiConnectExporter::new(config.anki_url)),
        media_extractor: Arc::new(FfmpegMediaExtractor),
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
