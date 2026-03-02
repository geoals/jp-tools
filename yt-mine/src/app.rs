use std::collections::HashSet;
use std::sync::Arc;

use axum::Router;
use axum::response::Html;
use axum::routing::{get, post};
use sqlx::SqlitePool;
use tower_http::services::ServeDir;

use jp_core::dictionary::Dictionary;
use jp_core::tokenize::Tokenizer;

use crate::routes::{api, vocab};
use crate::services::download::AudioDownloader;
use crate::services::export::AnkiExporter;
use crate::services::llm::LlmDefiner;
use crate::services::media::MediaExtractor;
use crate::services::transcribe::Transcriber;

const SPA_HTML: &str = include_str!("../templates/spa.html");
const STATIC_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/static");

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub downloader: Arc<dyn AudioDownloader>,
    pub transcriber: Arc<dyn Transcriber>,
    pub exporter: Arc<dyn AnkiExporter>,
    pub media_extractor: Arc<dyn MediaExtractor>,
    pub tokenizer: Arc<dyn Tokenizer>,
    pub dictionary_forms: Arc<HashSet<String>>,
    pub dictionaries: Vec<Arc<Dictionary>>,
    pub llm_definer: Option<Arc<dyn LlmDefiner>>,
    pub audio_dir: String,
    pub media_dir: String,
}

async fn spa_shell() -> Html<&'static str> {
    Html(SPA_HTML)
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(spa_shell))
        .route("/api/jobs", post(api::submit_job))
        .route("/api/{video_id}", get(api::get_job))
        .route("/api/{video_id}/status", get(api::poll_status))
        .route(
            "/api/{video_id}/sentences/{sentence_id}/preview",
            get(api::word_preview),
        )
        .route(
            "/api/{video_id}/sentences/{sentence_id}/llm-definition",
            get(api::llm_definition),
        )
        .route("/api/export", post(api::export_sentences))
        .route("/api/vocab/tokenize", post(vocab::tokenize_text))
        .route("/api/vocab/submit", post(vocab::submit_vocab))
        .route(
            "/{video_id}/sentences/{sentence_id}/audio",
            get(api::sentence_audio),
        )
        .route("/vocab", get(spa_shell))
        .route("/{video_id}", get(spa_shell))
        .nest_service("/static", ServeDir::new(STATIC_DIR))
        .with_state(state)
}
