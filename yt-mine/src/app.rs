use std::sync::Arc;

use axum::Router;
use axum::http::HeaderValue;
use axum::http::header::CACHE_CONTROL;
use axum::response::Html;
use axum::routing::{get, post};
use sqlx::SqlitePool;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

use jp_core::highlight::Highlighter;
use jp_core::knowledge::Knowledge;
use jp_core::tokenize::Tokenizer;

use crate::routes::api;
use crate::services::download::MediaDownloader;
use crate::services::export::AnkiExporter;
use crate::services::llm::LlmDefiner;
use crate::services::media::MediaExtractor;
use crate::services::transcribe::Transcriber;

const SPA_HTML: &str = include_str!("../templates/spa.html");
const STATIC_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/static");

/// Front-end code shared by more than one app in the workspace: the dictionary
/// popup, which the VN overlay and yt-mine both draw. Served from both, at the
/// same path, because there is no build step to copy it with — the two pages
/// load the identical file over HTTP.
const SHARED_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../web-shared");

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub downloader: Arc<dyn MediaDownloader>,
    pub transcriber: Arc<dyn Transcriber>,
    pub exporter: Arc<dyn AnkiExporter>,
    pub media_extractor: Arc<dyn MediaExtractor>,
    pub tokenizer: Arc<dyn Tokenizer>,
    /// The same pipeline as `tokenizer`, kept whole so a sentence's tokens can
    /// carry their ledger status and the popup can scan for other readings.
    /// `None` in fake mode, where the tokens are a mock's and there is no
    /// ledger to ask.
    pub highlighter: Option<Arc<Highlighter>>,
    /// The shared dictionary cache — what a word means, and how common it is.
    pub knowledge: Knowledge,
    /// For asking Anki directly — the popup's "already a card" badge and the
    /// link from it. The export path has its own client inside the exporter.
    pub http: reqwest::Client,
    pub anki_url: String,
    pub anki_vocab_field: Option<String>,
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
        .route("/api/videos", get(api::list_videos))
        .route("/api/{video_id}", get(api::get_job))
        .route("/api/{video_id}/status", get(api::poll_status))
        // The popup's own endpoints. Not nested under a video: what a word
        // means does not depend on where it was met.
        .route("/api/define", get(api::define))
        .route("/api/expand", get(api::expand))
        .route("/api/judge", post(api::judge))
        .route("/api/judge/many", post(api::judge_many))
        .route("/api/mined", get(api::mined))
        .route("/api/mined/browse", post(api::browse))
        .route("/api/export", post(api::export_sentences))
        .route(
            "/{video_id}/sentences/{sentence_id}/audio",
            get(api::sentence_audio),
        )
        .route("/{video_id}", get(spa_shell))
        .nest_service("/static", ServeDir::new(STATIC_DIR))
        .nest_service("/shared", ServeDir::new(SHARED_DIR))
        // The frontend has no build step, so nothing in a URL changes when a
        // module does. `no-cache` is revalidation, not "don't cache": the
        // browser keeps the bytes and `ServeDir` answers 304 off
        // `if-modified-since` until the file actually changes. read-stats
        // serves `/shared` under the same rule.
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ))
        .with_state(state)
}
