use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use sqlx::SqlitePool;

use crate::routes::mining;
use crate::services::download::AudioDownloader;
use crate::services::export::AnkiExporter;
use crate::services::llm::LlmDefiner;
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
    pub dictionaries: Vec<Arc<Dictionary>>,
    pub llm_definer: Option<Arc<dyn LlmDefiner>>,
    pub audio_dir: String,
    pub media_dir: String,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(mining::submit_page))
        .route("/mining/youtube", post(mining::submit_youtube))
        .route("/mining/export", post(mining::export_sentences))
        .route("/{video_id}", get(mining::video_page))
        .route("/{video_id}/status", get(mining::video_status_fragment))
        .route(
            "/{video_id}/sentences/{sentence_id}/audio",
            get(mining::sentence_audio),
        )
        .route(
            "/{video_id}/sentences/{sentence_id}/preview",
            get(mining::word_preview),
        )
        .route(
            "/{video_id}/sentences/{sentence_id}/llm-definition",
            get(mining::llm_definition),
        )
        .with_state(state)
}
