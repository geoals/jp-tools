use tracing::info;

use read_stats::app::{AppState, build_router};
use read_stats::config::Config;
use read_stats::db;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let config = Config::from_env();

    for path in [&config.db_path, &config.knowledge_db_path] {
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent).expect("failed to create database directory");
        }
    }
    let local = db::create_pool(&config.db_path)
        .await
        .expect("failed to open read-stats database");
    info!(path = %config.db_path, "read-stats database ready");

    let knowledge = db::open_knowledge(&config.knowledge_db_path)
        .await
        .expect("failed to open knowledge database");
    info!(path = %config.knowledge_db_path, "knowledge database ready");

    // One-time: settle what the old pause intervals covered, then drop the
    // table. Must run before anything reads the history, or a retired pause's
    // lines would count for one request.
    db::retire_pauses(&local, &knowledge)
        .await
        .expect("failed to retire the pauses table");

    let http = reqwest::Client::new();

    // Best-effort: re-download any cover whose file vanished since last run.
    tokio::spawn({
        let http = http.clone();
        let local = local.clone();
        let knowledge = knowledge.clone();
        let covers_dir = config.covers_dir.clone();
        async move {
            read_stats::services::covers::reconcile_missing(&http, &local, &knowledge, &covers_dir)
                .await
        }
    });

    let router = build_router(AppState {
        local,
        knowledge,
        covers_dir: config.covers_dir.clone(),
        http,
        anki_url: config.anki_url.clone(),
        anki_deck: config.anki_deck.clone(),
        anki_vocab_field: config.anki_vocab_field.clone(),
        anki_sentence_field: config.anki_sentence_field.clone(),
        anki_compact_def_field: config.anki_compact_def_field.clone(),
        auto_capture_on_add: config.auto_capture_on_add,
        sudachi_dict_path: config.sudachi_dict_path.clone(),
        vn_capture_script: config.vn_capture_script.clone(),
        anthropic_api_key: config.anthropic_api_key.clone(),
        whisper_url: config.whisper_url.clone(),
    });

    let listener = tokio::net::TcpListener::bind(&config.listen_addr)
        .await
        .expect("failed to bind listener");
    info!(addr = %config.listen_addr, "read-stats ready, listening");
    // with_connect_info exposes the client address so the Anki refresh can
    // probe the dashboard client for a local AnkiConnect first.
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .expect("server error");
}
