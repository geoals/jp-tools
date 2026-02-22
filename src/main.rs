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
use crate::services::transcribe::WhisperTranscriber;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config = Config::from_env();

    // Ensure audio directory exists
    std::fs::create_dir_all(&config.audio_dir).expect("failed to create audio directory");

    let pool = db::create_pool(&config.database_url())
        .await
        .expect("failed to create database pool");

    let state = AppState {
        db: pool,
        downloader: Arc::new(YtDlpDownloader),
        transcriber: Arc::new(WhisperTranscriber {
            script_path: config.transcribe_script,
        }),
        exporter: Arc::new(AnkiConnectExporter::new(config.anki_url)),
        audio_dir: config.audio_dir,
    };

    let router = build_router(state);

    let listener = tokio::net::TcpListener::bind(&config.listen_addr)
        .await
        .expect("failed to bind listener");

    info!(addr = %config.listen_addr, "server starting");
    axum::serve(listener, router).await.expect("server error");
}
