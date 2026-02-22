use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use sqlx::SqlitePool;

use crate::routes::mining;
use crate::services::download::AudioDownloader;
use crate::services::export::AnkiExporter;
use crate::services::media::MediaExtractor;
use crate::services::dictionary::Dictionary;
use crate::services::tokenize::Tokenizer;
use crate::services::transcribe::Transcriber;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub downloader: Arc<dyn AudioDownloader>,
    pub transcriber: Arc<dyn Transcriber>,
    pub exporter: Arc<dyn AnkiExporter>,
    pub media_extractor: Arc<dyn MediaExtractor>,
    pub tokenizer: Arc<dyn Tokenizer>,
    pub dictionary: Option<Arc<Dictionary>>,
    pub audio_dir: String,
    pub media_dir: String,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/mining", get(mining::submit_page))
        .route("/mining/youtube", post(mining::submit_youtube))
        .route("/mining/jobs/{id}", get(mining::job_page))
        .route("/mining/jobs/{id}/status", get(mining::job_status_fragment))
        .route(
            "/mining/jobs/{job_id}/sentences/{sentence_id}/audio",
            get(mining::sentence_audio),
        )
        .route("/mining/export", post(mining::export_sentences))
        .with_state(state)
}
