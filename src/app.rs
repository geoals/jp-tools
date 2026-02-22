use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use sqlx::SqlitePool;

use crate::routes::mining;
use crate::services::download::AudioDownloader;
use crate::services::export::AnkiExporter;
use crate::services::transcribe::Transcriber;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub downloader: Arc<dyn AudioDownloader>,
    pub transcriber: Arc<dyn Transcriber>,
    pub exporter: Arc<dyn AnkiExporter>,
    pub audio_dir: String,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/mining", get(mining::submit_page))
        .route("/mining/youtube", post(mining::submit_youtube))
        .route("/mining/jobs/{id}", get(mining::job_page))
        .route("/mining/jobs/{id}/status", get(mining::job_status_fragment))
        .route("/mining/export", post(mining::export_sentences))
        .with_state(state)
}
