use std::path::PathBuf;

use axum::Router;
use axum::extract::State;
use axum::http::header::CACHE_CONTROL;
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::{delete, get, post};
use jp_core::knowledge::Knowledge;
use sqlx::SqlitePool;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::routes::ankiproxy;
use crate::routes::{
    anki, books, days, ingest, kanji, lookups, reader, sessions, settings, summary, timeline,
    tokenize, vocab, works,
};

const SPA_HTML: &str = include_str!("../templates/spa.html");
fn static_dir() -> PathBuf {
    jp_core::install::install_root().join("kotodex-server/static")
}

/// The VN overlay's page, and the launcher that shows it.
///
/// A second reading surface over this same API — the line feed, the dictionary,
/// the ledger, the card — drawn over the game instead of beside it. It shares no
/// code with the dashboard's own frontend, which is why it is its own directory
/// rather than part of `static`, but it is this app's page: every route it calls
/// is one of these, and none of them can be answered anywhere else.
///
/// The Qt shell that puts it over a fullscreen window is `layer-overlay`, which
/// knows nothing about reading. See `overlay/vn-overlay.py`.
fn overlay_dir() -> PathBuf {
    jp_core::install::install_root().join("kotodex-server/overlay")
}

/// Front-end code shared by more than one app in the workspace: the dictionary
/// popup, which the VN overlay and yt-mine both draw. Served from both, at the
/// same path, because there is no build step to copy it with — the two pages
/// load the identical file over HTTP.
fn shared_dir() -> PathBuf {
    jp_core::install::install_root().join("web-shared")
}

#[derive(Clone)]
pub struct AppState {
    /// kotodex-server's own database: settings, reader marks, cover sources.
    pub local: SqlitePool,
    /// jp-core's shared knowledge database: the line stream, works, manual
    /// sessions, the Anki mirror, lookups — and the dictionary cache. A
    /// distinct type, so passing the wrong database is a compile error.
    pub knowledge: Knowledge,
    pub covers_dir: std::path::PathBuf,
    pub http: reqwest::Client,
    pub anki_url: String,
    pub anki_deck: String,
    /// The note type and every field name on it. `mine` builds a card from
    /// this; the named fields below are the same map, read often enough to be
    /// worth their own names.
    pub anki: jp_mine_core::config::AnkiConfig,
    pub anki_vocab_field: String,
    pub anki_sentence_field: String,
    pub anki_compact_def_field: String,
    pub auto_capture_on_add: bool,
    pub sudachi_dict_path: std::path::PathBuf,
    pub vn_capture_script: std::path::PathBuf,
    /// `KOTODEX_ANTHROPIC_API_KEY`, the fallback behind the stored key. Read
    /// through [`crate::services::llm::provider`] and nowhere else. `setup.sh`
    /// writes this one.
    pub env_api_key: Option<String>,
    /// whisper-service base URL, probed for the reader's trim-status indicator.
    pub whisper_url: String,
    /// The Local Audio Server for Yomitan, proxied for the popup's 🔊.
    pub local_audio_url: String,
    /// The reading view's Sudachi pipeline, built on the first line that needs
    /// it and shared from then on. See [`reader::highlight::Shared`].
    pub highlighter: reader::highlight::Shared,
    /// Public demo: serve the seed, change nothing. See [`demo_guard`].
    pub demo: bool,
}

/// Refuse everything that could write, on the public demo.
///
/// One gate over the whole router rather than a check per handler: the demo has
/// to stay safe as routes are added, and a new POST that nobody remembered to
/// guard is exactly the failure a shared instance cannot have. GET is the whole
/// dashboard — every figure on it is derived at query time — so refusing the
/// rest costs the demo nothing.
///
/// Not done by opening the databases read-only, because the migrations in a new
/// release still have to run against the scratch copy at boot.
async fn demo_guard(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let method = request.method();
    if state.demo && method != Method::GET && method != Method::HEAD {
        return (
            StatusCode::FORBIDDEN,
            "Read-only demo — this would change the data, so it is not saved.",
        )
            .into_response();
    }
    next.run(request).await
}

async fn spa_shell() -> Html<&'static str> {
    Html(SPA_HTML)
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(spa_shell))
        .route("/api/summary", get(summary::summary))
        .route("/api/days", get(days::days))
        .route(
            "/api/sessions",
            get(sessions::list_sessions).post(sessions::create_session),
        )
        .route("/api/sessions/{id}", delete(sessions::delete_session))
        .route("/api/sessions/{id}/content", get(sessions::session_content))
        .route("/api/text/count", post(sessions::count_text))
        .route("/api/tokenize", post(tokenize::tokenize_text))
        .route("/api/day/timeline", get(timeline::day_timeline))
        .route("/api/books", get(books::list_books))
        // An epub is megabytes, past axum's 2 MB default.
        .route(
            "/api/books/upload",
            post(books::upload_book).layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024)),
        )
        .route("/api/books/setup", post(books::setup_book))
        .route("/api/books/preview", post(books::preview_book))
        .route("/api/books/log", post(books::log_book))
        .route("/api/books/skip", post(books::skip_book))
        .route("/api/works", get(works::works).post(works::upsert_work))
        .route("/api/works/detail", get(works::work_detail))
        // VNDB by title, so adding a work is not a trip to a website for an id.
        .route("/api/works/search", get(works::search_works))
        .route("/api/works/triage", get(works::work_triage))
        .route(
            "/api/works/{id}",
            axum::routing::put(works::update_work).delete(works::delete_work),
        )
        .nest_service("/covers", ServeDir::new(state.covers_dir.clone()))
        .route(
            "/api/capture/pause",
            axum::routing::post(settings::toggle_capture),
        )
        .route("/api/anki/refresh", axum::routing::post(anki::anki_refresh))
        .route("/api/anki/summary", get(anki::anki_summary))
        .route("/api/anki/cards", get(anki::anki_cards))
        .route("/api/anki/up", get(anki::anki_up))
        .route("/api/vocab/summary", get(vocab::vocab_summary))
        .route("/api/vocab/history", get(vocab::vocab_history))
        .route("/api/vocab/queue", get(vocab::vocab_queue))
        .route("/api/vocab/judge", axum::routing::post(vocab::vocab_judge))
        .route("/api/vocab/browse", get(vocab::vocab_browse))
        .route("/api/vocab/surfaces", get(vocab::vocab_surfaces))
        .route("/api/vocab/non-words", get(vocab::vocab_non_words))
        .route(
            "/api/vocab/blacklist-non-words",
            axum::routing::post(vocab::vocab_blacklist_non_words),
        )
        .route(
            "/api/vocab/anki-import",
            axum::routing::post(vocab::vocab_anki_import),
        )
        .route(
            "/api/vocab/repair-empty-readings",
            axum::routing::post(vocab::vocab_repair_empty_readings),
        )
        .route(
            "/api/vocab/rebuild",
            axum::routing::post(vocab::vocab_rebuild),
        )
        .route("/api/lookups/summary", get(lookups::lookups_summary))
        .route("/api/kanji", get(kanji::kanji))
        .route(
            "/api/settings",
            get(settings::get_settings).put(settings::put_settings),
        )
        // Its own route because the value is write-only: see `put_llm_key`.
        .route(
            "/api/settings/llm-key",
            axum::routing::put(settings::put_llm_key),
        )
        // The capability matrix, past its cache — what "check again" asks after
        // the reader has done something outside the app.
        .route("/api/setup", get(reader::capabilities::setup))
        // Where every source hands its text over, from any machine.
        .route("/api/lines", axum::routing::post(ingest::ingest_lines))
        .route(
            "/api/lines/retract",
            axum::routing::post(ingest::retract_line),
        )
        .route("/api/lines/stream", get(reader::stream::lines_stream))
        .route("/api/lines/before", get(reader::lines::lines_before))
        .route("/api/reader/state", get(reader::state::reader_state))
        .route("/api/reader/define", get(reader::define::define))
        .route("/api/reader/expand", get(reader::define::expand))
        .route(
            "/api/reader/lookup/retract",
            axum::routing::post(reader::define::retract),
        )
        .route("/api/reader/mine", axum::routing::post(reader::mine::mine))
        .route("/api/reader/mined", get(reader::mined::mined))
        .route("/api/reader/fonts", get(reader::fonts::fonts))
        .route("/api/reader/audio", get(reader::audio::audio))
        .route("/api/reader/audio/clip", get(reader::audio::clip))
        .route(
            "/api/reader/mined/browse",
            axum::routing::post(reader::mined::browse),
        )
        .route(
            "/api/lines/discard",
            axum::routing::post(reader::lines::discard_lines),
        )
        .route(
            "/api/lines/undiscard",
            axum::routing::post(reader::lines::undiscard_lines),
        )
        .route("/api/vn/windows", get(reader::capture::vn_windows))
        .route(
            "/api/vn/window",
            get(reader::capture::vn_window).put(reader::capture::set_vn_window),
        )
        .route(
            "/api/reader/explain",
            axum::routing::post(reader::explain::explain_line),
        )
        // Yomitan's AnkiConnect endpoint: forwards to Anki, counts lookups.
        .route(
            "/anki-proxy",
            axum::routing::post(ankiproxy::proxy).options(ankiproxy::preflight),
        )
        .nest_service("/static", ServeDir::new(static_dir()))
        .nest_service("/overlay", ServeDir::new(overlay_dir()))
        .nest_service("/shared", ServeDir::new(shared_dir()))
        // Frontend has no build step / cache busting — force revalidation so
        // browsers never serve stale modules.
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            demo_guard,
        ))
        .with_state(state)
}
