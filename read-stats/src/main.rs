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

    let router = build_router(AppState { pool });

    let listener = tokio::net::TcpListener::bind(&config.listen_addr)
        .await
        .expect("failed to bind listener");
    info!(addr = %config.listen_addr, "read-stats ready, listening");
    axum::serve(listener, router).await.expect("server error");
}
