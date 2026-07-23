use axum::Router;
use axum::http::HeaderValue;
use axum::http::header::CACHE_CONTROL;
use axum::response::Html;
use axum::routing::{delete, get};
use sqlx::SqlitePool;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::ankiproxy;
use crate::routes::{api, reader};

const SPA_HTML: &str = include_str!("../templates/spa.html");
const STATIC_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/static");

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub covers_dir: std::path::PathBuf,
    pub http: reqwest::Client,
    pub anki_url: String,
    pub anki_deck: String,
    pub anki_vocab_field: String,
    pub sudachi_dict_path: std::path::PathBuf,
    pub vn_capture_script: std::path::PathBuf,
    pub anthropic_api_key: Option<String>,
    pub llm_model: String,
    /// whisper-service base URL, probed for the reader's trim-status indicator.
    pub whisper_url: String,
}

async fn spa_shell() -> Html<&'static str> {
    Html(SPA_HTML)
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(spa_shell))
        .route("/api/summary", get(api::summary))
        .route("/api/days", get(api::days))
        .route(
            "/api/sessions",
            get(api::list_sessions).post(api::create_session),
        )
        .route("/api/sessions/{id}", delete(api::delete_session))
        .route("/api/day/timeline", get(api::day_timeline))
        .route("/api/works", get(api::works).post(api::upsert_work))
        .route(
            "/api/works/{id}",
            axum::routing::put(api::update_work).delete(api::delete_work),
        )
        .nest_service("/covers", ServeDir::new(state.covers_dir.clone()))
        .route("/api/pause", axum::routing::post(api::toggle_pause))
        .route("/api/anki/refresh", axum::routing::post(api::anki_refresh))
        .route("/api/anki/summary", get(api::anki_summary))
        .route("/api/lookups/summary", get(api::lookups_summary))
        .route("/api/dialogue/summary", get(api::dialogue_summary))
        .route(
            "/api/settings",
            get(api::get_settings).put(api::put_settings),
        )
        // Reading view (phone): live line feed + the mine trigger.
        .route("/api/lines/stream", get(reader::lines_stream))
        .route("/api/reader/state", get(reader::reader_state))
        .route(
            "/api/lines/discard",
            axum::routing::post(reader::discard_lines),
        )
        .route(
            "/api/lines/undiscard",
            axum::routing::post(reader::undiscard_lines),
        )
        .route("/api/vn/capture", axum::routing::post(reader::vn_capture))
        .route("/api/vn/windows", get(reader::vn_windows))
        .route(
            "/api/reader/explain",
            axum::routing::post(reader::explain_line),
        )
        // Yomitan's AnkiConnect endpoint: forwards to Anki, counts lookups.
        .route(
            "/anki-proxy",
            axum::routing::post(ankiproxy::proxy).options(ankiproxy::preflight),
        )
        .nest_service("/static", ServeDir::new(STATIC_DIR))
        // Frontend has no build step / cache busting — force revalidation so
        // browsers never serve stale modules.
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ))
        .with_state(state)
}
