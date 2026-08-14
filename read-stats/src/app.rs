use axum::Router;
use axum::http::HeaderValue;
use axum::http::header::CACHE_CONTROL;
use axum::response::Html;
use axum::routing::{delete, get, post};
use jp_core::knowledge::Knowledge;
use sqlx::SqlitePool;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::routes::ankiproxy;
use crate::routes::{
    anki, days, kanji, lookups, reader, sessions, settings, summary, timeline, tokenize, vocab,
    works,
};

const SPA_HTML: &str = include_str!("../templates/spa.html");
const STATIC_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/static");

/// The overlay's page, which belongs to vn-mine and is only served from here.
///
/// It shares no code with this app's own frontend — no imports, no stylesheet —
/// so it lives with the Qt shell that shows it. Served from read-stats anyway
/// because it needs this origin: it calls eight `/api` routes with relative
/// URLs, and a `file://` origin resolves those against the filesystem. Serving
/// it here is what keeps the overlay a client of this API rather than a second
/// copy of the dictionary and the ledger.
const OVERLAY_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../vn-mine/overlay");

/// Front-end code shared by more than one app in the workspace: the dictionary
/// popup, which the VN overlay and yt-mine both draw. Served from both, at the
/// same path, because there is no build step to copy it with — the two pages
/// load the identical file over HTTP.
const SHARED_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../web-shared");

#[derive(Clone)]
pub struct AppState {
    /// read-stats' own database: settings, reader marks, cover sources.
    pub local: SqlitePool,
    /// jp-core's shared knowledge database: the line stream, works, manual
    /// sessions, the Anki mirror, lookups — and the dictionary cache. A
    /// distinct type, so passing the wrong database is a compile error.
    pub knowledge: Knowledge,
    pub covers_dir: std::path::PathBuf,
    pub http: reqwest::Client,
    pub anki_url: String,
    pub anki_deck: String,
    pub anki_vocab_field: String,
    pub anki_sentence_field: String,
    pub anki_compact_def_field: String,
    pub auto_capture_on_add: bool,
    pub sudachi_dict_path: std::path::PathBuf,
    pub vn_capture_script: std::path::PathBuf,
    pub anthropic_api_key: Option<String>,
    /// whisper-service base URL, probed for the reader's trim-status indicator.
    pub whisper_url: String,
    /// The reading view's Sudachi pipeline, built on the first line that needs
    /// it and shared from then on. See [`reader::highlight::Shared`].
    pub highlighter: reader::highlight::Shared,
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
        .route("/api/works", get(works::works).post(works::upsert_work))
        .route("/api/works/detail", get(works::work_detail))
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
        // The export is a few megabytes of JSON — past axum's 2 MB default,
        // which rejects it as a bare 413 with nothing in the log to explain
        // why. The limit is raised on this route alone.
        .route(
            "/api/vocab/jiten-import",
            axum::routing::post(vocab::vocab_jiten_import)
                .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024)),
        )
        .route(
            "/api/vocab/repair-empty-readings",
            axum::routing::post(vocab::vocab_repair_empty_readings),
        )
        .route(
            "/api/vocab/promotion-queue",
            get(vocab::vocab_promotion_queue),
        )
        .route(
            "/api/vocab/promote",
            axum::routing::post(vocab::vocab_promote),
        )
        .route(
            "/api/vocab/frequency-summary",
            get(vocab::vocab_frequency_summary),
        )
        .route(
            "/api/vocab/frequency-queue",
            get(vocab::vocab_frequency_queue),
        )
        .route(
            "/api/vocab/frequency-commit",
            axum::routing::post(vocab::vocab_frequency_commit),
        )
        .route(
            "/api/vocab/rebuild",
            axum::routing::post(vocab::vocab_rebuild),
        )
        .route(
            "/api/vocab/seed-undo",
            axum::routing::post(vocab::vocab_seed_undo),
        )
        .route("/api/lookups/summary", get(lookups::lookups_summary))
        .route("/api/kanji", get(kanji::kanji))
        .route(
            "/api/settings",
            get(settings::get_settings).put(settings::put_settings),
        )
        // Reading view: live line feed + the mine trigger.
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
        .route(
            "/api/vn/capture",
            axum::routing::post(reader::capture::vn_capture),
        )
        .route("/api/vn/windows", get(reader::capture::vn_windows))
        .route(
            "/api/reader/explain",
            axum::routing::post(reader::explain::explain_line),
        )
        // Yomitan's AnkiConnect endpoint: forwards to Anki, counts lookups.
        .route(
            "/anki-proxy",
            axum::routing::post(ankiproxy::proxy).options(ankiproxy::preflight),
        )
        .nest_service("/static", ServeDir::new(STATIC_DIR))
        .nest_service("/overlay", ServeDir::new(OVERLAY_DIR))
        .nest_service("/shared", ServeDir::new(SHARED_DIR))
        // Frontend has no build step / cache busting — force revalidation so
        // browsers never serve stale modules.
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ))
        .with_state(state)
}
