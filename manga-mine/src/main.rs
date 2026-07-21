use std::sync::Arc;

use tracing::info;

use jp_core::dictionary::Dictionary;
use jp_core::tokenize::{SudachiTokenizer, Tokenizer};
use jp_mine_core::export::{AnkiConnectExporter, AnkiExporter};

use manga_mine::app::{AppState, build_router};
use manga_mine::config::Config;
use manga_mine::services::fake::{FakeAnkiExporter, FakeOcrEngine, FakeTokenizer};
use manga_mine::services::ocr::{MangaOcrEngine, OcrEngine};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let config = Config::from_env();

    // Ensure directories exist
    std::fs::create_dir_all(&config.inbox_dir).expect("failed to create inbox directory");
    std::fs::create_dir_all(&config.media_dir).expect("failed to create media directory");
    let inbox_dir =
        std::fs::canonicalize(&config.inbox_dir).expect("failed to resolve inbox directory");
    let media_dir = std::fs::canonicalize(&config.media_dir)
        .expect("failed to resolve media directory")
        .to_string_lossy()
        .into_owned();

    let (tokenizer, dictionaries, ocr, exporter): (
        Arc<dyn Tokenizer>,
        Vec<Arc<Dictionary>>,
        Arc<dyn OcrEngine>,
        Arc<dyn AnkiExporter>,
    ) = if config.fake_api {
        info!("*** DEV MODE — using fake services (no external deps needed) ***");
        (
            Arc::new(FakeTokenizer),
            vec![],
            Arc::new(FakeOcrEngine),
            Arc::new(FakeAnkiExporter),
        )
    } else {
        // Dictionary cache lives in the shared SQLite DB (imported by yt-mine
        // or on first run here)
        let pool = jp_core_pool(&config.database_url()).await;

        let mut dictionaries: Vec<Arc<Dictionary>> = Vec::new();
        for path in &config.dictionary_paths {
            info!(path, "loading dictionary");
            let dict = Dictionary::load_or_import(&pool, std::path::Path::new(path))
                .await
                .expect("failed to load dictionary");
            dictionaries.push(Arc::new(dict));
        }
        if dictionaries.is_empty() {
            info!(
                "no dictionaries configured (set JP_TOOLS_DICTIONARY_PATHS to enable definitions)"
            );
        }

        let headwords = jp_core::db::get_all_headwords(&pool)
            .await
            .expect("failed to load headwords");
        if !headwords.is_empty() {
            info!(
                count = headwords.len(),
                "loaded headwords for dictionary-aware tokenization"
            );
        }

        let tokenizer: Arc<dyn Tokenizer> = Arc::new(
            SudachiTokenizer::new(&config.sudachi_dict_path, headwords)
                .expect("failed to initialize tokenizer"),
        );
        info!("Sudachi tokenizer ready");

        info!(url = %config.ocr_service_url, "using manga-ocr service");
        (
            tokenizer,
            dictionaries,
            Arc::new(MangaOcrEngine::new(config.ocr_service_url.clone())),
            Arc::new(AnkiConnectExporter::new(
                config.anki_url.clone(),
                config.anki.clone(),
            )),
        )
    };

    let state = AppState {
        tokenizer,
        dictionaries,
        ocr,
        exporter,
        inbox_dir,
        media_dir,
        anki_config: config.anki,
        use_client_anki: config.use_client_anki && !config.fake_api,
        card_image_max_dim: config.card_image_max_dim,
        card_image_quality: config.card_image_quality,
        client_anki_cache: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
    };

    let router = build_router(state);

    let listener = tokio::net::TcpListener::bind(&config.listen_addr)
        .await
        .expect("failed to bind listener");

    info!(addr = %config.listen_addr, "manga-mine ready, listening");
    // with_connect_info exposes the client address so exports can detect a
    // client-side AnkiConnect (phone)
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .expect("server error");
}

async fn jp_core_pool(database_url: &str) -> sqlx::SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .connect(database_url)
        .await
        .expect("failed to open dictionary database");
    jp_core::db::run_migrations(&pool)
        .await
        .expect("failed to run dictionary migrations");
    pool
}
