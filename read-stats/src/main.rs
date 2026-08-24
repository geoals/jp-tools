use tracing::{info, warn};

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

    let knowledge = db::open_knowledge(&config.knowledge_db_path, !config.demo)
        .await
        .expect("failed to open knowledge database");
    info!(path = %config.knowledge_db_path, "knowledge database ready");

    // One-time: settle what the old pause intervals covered, then drop the
    // table. Must run before anything reads the history, or a retired pause's
    // lines would count for one request.
    if !config.demo {
        db::retire_pauses(&local, &knowledge)
            .await
            .expect("failed to retire the pauses table");
    }

    // Best-effort, and off the boot path: attach JMdict entry ids to any
    // dictionary cached before they were stored. They are what tells the
    // vocabulary count that 叔父, 伯父 and おじ are one word
    // (`jp_core::knowledge::lexeme`). Until it finishes, the count is merely
    // conservative — it over-reports by a few dozen spellings — so nothing
    // needs to wait for it. Parsing Jitendex takes a while and runs once.
    if !config.demo {
        tokio::spawn({
        let knowledge = knowledge.clone();
        async move {
            match jp_core::dictionary::Dictionary::backfill_sequences(knowledge.pool()).await {
                Ok(0) => {}
                Ok(n) => info!(updated = n, "dictionary entry ids backfilled"),
                Err(e) => warn!(error = %e, "could not backfill dictionary entry ids"),
            }
        }
        });
    }

    let http = reqwest::Client::new();

    // Best-effort: re-download any cover whose file vanished since last run.
    if !config.demo {
        tokio::spawn({
            let http = http.clone();
            let local = local.clone();
            let knowledge = knowledge.clone();
            let covers_dir = config.covers_dir.clone();
            async move {
                read_stats::services::covers::reconcile_missing(
                    &http,
                    &local,
                    &knowledge,
                    &covers_dir,
                )
                .await
            }
        });
    }

    let state = AppState {
        local,
        knowledge,
        covers_dir: config.covers_dir.clone(),
        http,
        anki_url: config.anki_url.clone(),
        anki_deck: config.anki_deck.clone(),
        anki: config.anki.clone(),
        anki_vocab_field: config.anki_vocab_field.clone(),
        anki_sentence_field: config.anki_sentence_field.clone(),
        anki_compact_def_field: config.anki_compact_def_field.clone(),
        auto_capture_on_add: config.auto_capture_on_add,
        sudachi_dict_path: config.sudachi_dict_path.clone(),
        vn_capture_script: config.vn_capture_script.clone(),
        anthropic_api_key: config.anthropic_api_key.clone(),
        whisper_url: config.whisper_url.clone(),
        local_audio_url: config.local_audio_url.clone(),
        highlighter: Default::default(),
        demo: config.demo,
    };

    // Off the startup path, not on it: the dictionary load is seconds of CPU
    // that a dashboard-only start should not wait for, and the reader's first
    // popup should not pay for either.
    if !config.demo {
        read_stats::routes::reader::highlight::warm(state.clone());
    }

    let router = build_router(state);

    let listener = tokio::net::TcpListener::bind(&config.listen_addr)
        .await
        .expect("failed to bind listener");
    if config.demo {
        info!("demo mode — every request that is not a GET is refused");
    }
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
