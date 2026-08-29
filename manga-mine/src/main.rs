use std::sync::Arc;

use tracing::info;

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

    std::fs::create_dir_all(&config.inbox_dir).expect("failed to create inbox directory");
    std::fs::create_dir_all(&config.media_dir).expect("failed to create media directory");
    let inbox_dir =
        std::fs::canonicalize(&config.inbox_dir).expect("failed to resolve inbox directory");
    let media_dir = std::fs::canonicalize(&config.media_dir)
        .expect("failed to resolve media directory")
        .to_string_lossy()
        .into_owned();

    // The dictionary cache lives in the shared knowledge database, and
    // `jp-dict` is what fills it. Read here, never imported: a service that
    // parses zips becomes a prerequisite of the other services.
    let knowledge = jp_core::knowledge::Knowledge::open(&config.knowledge_db_path)
        .await
        .expect("failed to open knowledge database");

    let (tokenizer, ocr, exporter): (
        Arc<dyn Tokenizer>,
        Arc<dyn OcrEngine>,
        Arc<dyn AnkiExporter>,
    ) = if config.fake_api {
        info!("*** DEV MODE — using fake services (no external deps needed) ***");
        (
            Arc::new(FakeTokenizer),
            Arc::new(FakeOcrEngine),
            Arc::new(FakeAnkiExporter),
        )
    } else {
        let headwords = jp_core::knowledge::dictionaries::get_all_headwords(knowledge.pool())
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
            Arc::new(MangaOcrEngine::new(config.ocr_service_url.clone())),
            Arc::new(AnkiConnectExporter::new(
                config.anki_url.clone(),
                config.anki.clone(),
            )),
        )
    };

    let state = AppState {
        tokenizer,
        knowledge,
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
