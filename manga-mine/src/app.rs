use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::HeaderValue;
use axum::http::header::CACHE_CONTROL;
use axum::response::Html;
use axum::routing::{get, post};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

use jp_core::dictionary::Dictionary;
use jp_core::tokenize::Tokenizer;
use jp_mine_core::config::AnkiConfig;
use jp_mine_core::export::AnkiExporter;

use crate::routes::api;
use crate::services::ocr::OcrEngine;

const SPA_HTML: &str = include_str!("../templates/spa.html");
const STATIC_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/static");

#[derive(Clone)]
pub struct AppState {
    pub tokenizer: Arc<dyn Tokenizer>,
    pub dictionaries: Vec<Arc<Dictionary>>,
    pub ocr: Arc<dyn OcrEngine>,
    pub exporter: Arc<dyn AnkiExporter>,
    /// Watched inbox folder = the mining queue.
    pub inbox_dir: PathBuf,
    /// Directory for compressed card images awaiting Anki upload.
    pub media_dir: String,
    /// Anki note type / field mapping, needed to build a per-request exporter
    /// when the client's own AnkiConnect is used.
    pub anki_config: AnkiConfig,
    /// Probe the requesting client for AnkiConnect and prefer it (real mode only).
    pub use_client_anki: bool,
    /// Card image compression: longest side in pixels and JPEG quality.
    pub card_image_max_dim: u32,
    pub card_image_quality: u8,
}

async fn spa_shell() -> Html<&'static str> {
    Html(SPA_HTML)
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(spa_shell))
        .route("/p/{name}", get(spa_shell))
        .route("/api/queue", get(api::list_queue))
        .route("/api/photos", post(api::upload_photo))
        .route("/api/photos/{name}", get(api::get_photo))
        .route("/api/photos/{name}/thumb", get(api::get_thumbnail))
        .route("/api/photos/{name}/ocr", post(api::ocr_crop))
        .route("/api/photos/{name}/mark", post(api::mark_photo))
        .route("/api/preview", get(api::word_preview))
        .route("/api/sources", get(api::list_sources))
        .route("/api/export", post(api::export_card))
        .nest_service("/static", ServeDir::new(STATIC_DIR))
        // Phone photos can be large (12MP JPEG ≈ 5–10 MB)
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024))
        // Frontend has no build step / cache busting — force revalidation so
        // browsers never serve stale modules. Photo/thumb handlers set their
        // own max-age, which if_not_present leaves alone.
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ))
        .with_state(state)
}
