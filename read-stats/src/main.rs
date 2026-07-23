use tracing::info;

use read_stats::app::{AppState, build_router};
use read_stats::config::Config;
use read_stats::db;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let config = Config::from_env();

    if let Some(parent) = std::path::Path::new(&config.db_path).parent() {
        std::fs::create_dir_all(parent).expect("failed to create stats DB directory");
    }
    let pool = db::create_pool(&config.db_path)
        .await
        .expect("failed to open stats database");
    info!(path = %config.db_path, "stats database ready");

    let router = build_router(AppState {
        pool,
        covers_dir: config.covers_dir.clone(),
        http: reqwest::Client::new(),
        anki_url: config.anki_url.clone(),
        anki_deck: config.anki_deck.clone(),
        anki_vocab_field: config.anki_vocab_field.clone(),
        sudachi_dict_path: config.sudachi_dict_path.clone(),
        vn_capture_script: config.vn_capture_script.clone(),
        anthropic_api_key: config.anthropic_api_key.clone(),
        llm_model: config.llm_model.clone(),
        whisper_url: config.whisper_url.clone(),
    });

    let listener = tokio::net::TcpListener::bind(&config.listen_addr)
        .await
        .expect("failed to bind listener");
    info!(addr = %config.listen_addr, "read-stats ready, listening");
    // with_connect_info exposes the client address so the Anki refresh can
    // probe the dashboard client (phone) for a local AnkiConnect first.
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .expect("server error");
}
