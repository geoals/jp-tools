use std::sync::Arc;

use tracing::info;

use jp_core::dictionary::Dictionary;
use jp_core::tokenize::{SudachiTokenizer, Tokenizer};

use yt_mine::app::{AppState, build_router};
use yt_mine::config::Config;
use yt_mine::db;
use yt_mine::services::download::{AudioDownloader, YtDlpDownloader};
use yt_mine::services::export::{AnkiConnectExporter, AnkiExporter};
use yt_mine::services::fake::{
    FakeAnkiExporter, FakeDownloader, FakeLlmDefiner, FakeMediaExtractor, FakeTokenizer,
    FakeTranscriber,
};
use yt_mine::services::llm::{AnthropicDefiner, LlmDefiner};
use yt_mine::services::media::{FfmpegMediaExtractor, MediaExtractor};
use yt_mine::services::transcribe::{RemoteTranscriber, Transcriber};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let config = Config::from_env();

    // Ensure output directories exist
    std::fs::create_dir_all(&config.audio_dir).expect("failed to create audio directory");
    std::fs::create_dir_all(&config.media_dir).expect("failed to create media directory");
    let media_dir = std::fs::canonicalize(&config.media_dir)
        .expect("failed to resolve media directory")
        .to_string_lossy()
        .into_owned();

    let pool = db::create_pool(&config.database_url())
        .await
        .expect("failed to create database pool");

    let deleted = db::delete_incomplete_jobs(&pool)
        .await
        .expect("failed to clean up incomplete jobs");
    if deleted > 0 {
        info!(
            count = deleted,
            "cleaned up incomplete jobs from previous run"
        );
    }

    // Dictionaries live in the shared knowledge database, not in yt-mine's
    // own — every tool asks the same one whether a string is a word.
    let knowledge = jp_core::knowledge::Knowledge::open(&config.knowledge_db_path)
        .await
        .expect("failed to open knowledge database");
    info!(path = %config.knowledge_db_path, "knowledge database ready");

    // Cached only — importing a zip belongs to `jp-dict`, not to a service.
    // Owned here, it made yt-mine a prerequisite of every other tool: a
    // dictionary added for the VN overlay stayed invisible until yt-mine
    // happened to boot.
    let dictionaries: Vec<Arc<Dictionary>> = Dictionary::load_cached(knowledge.pool())
        .await
        .expect("failed to load dictionaries")
        .into_iter()
        .map(Arc::new)
        .collect();
    if dictionaries.is_empty() {
        info!("no dictionaries cached (run `jp-dict sync` to enable definitions)");
    } else {
        info!(
            count = dictionaries.len(),
            "dictionaries ready (cached in db)"
        );
    }

    let headwords = jp_core::knowledge::dictionaries::get_all_headwords(knowledge.pool())
        .await
        .expect("failed to load headwords");
    if !headwords.is_empty() {
        info!(
            count = headwords.len(),
            "loaded headwords for dictionary-aware tokenization"
        );
    }

    let (downloader, transcriber, exporter, media_extractor, tokenizer): (
        Arc<dyn AudioDownloader>,
        Arc<dyn Transcriber>,
        Arc<dyn AnkiExporter>,
        Arc<dyn MediaExtractor>,
        Arc<dyn Tokenizer>,
    ) = if config.fake_api {
        info!("*** DEV MODE — using fake services (no external deps needed) ***");
        (
            Arc::new(FakeDownloader),
            Arc::new(FakeTranscriber),
            Arc::new(FakeAnkiExporter),
            Arc::new(FakeMediaExtractor),
            Arc::new(FakeTokenizer),
        )
    } else {
        // Tokenizer gets terms-only headwords for Mode C compound validation
        let tokenizer: Arc<dyn Tokenizer> = Arc::new(
            SudachiTokenizer::new(&config.sudachi_dict_path, headwords)
                .expect("failed to initialize tokenizer"),
        );
        info!("Sudachi tokenizer ready");

        info!(url = %config.whisper_service_url, "using remote whisper service");
        let transcriber: Arc<dyn Transcriber> =
            Arc::new(RemoteTranscriber::new(config.whisper_service_url.clone()));

        (
            Arc::new(YtDlpDownloader),
            transcriber,
            Arc::new(AnkiConnectExporter::new(config.anki_url, config.anki)),
            Arc::new(FfmpegMediaExtractor),
            tokenizer,
        )
    };

    let llm_definer: Option<Arc<dyn LlmDefiner>> = if config.fake_api {
        Some(Arc::new(FakeLlmDefiner))
    } else {
        config.anthropic_api_key.as_ref().map(|key| {
            info!("LLM definitions enabled (model: {})", config.llm_model);
            Arc::new(AnthropicDefiner::new(key.clone(), config.llm_model.clone()))
                as Arc<dyn LlmDefiner>
        })
    };

    let state = AppState {
        db: pool,
        downloader,
        transcriber,
        exporter,
        media_extractor,
        tokenizer,
        dictionaries,
        llm_definer,
        audio_dir: config.audio_dir,
        media_dir,
    };

    let router = build_router(state);

    let listener = tokio::net::TcpListener::bind(&config.listen_addr)
        .await
        .expect("failed to bind listener");

    info!(addr = %config.listen_addr, "server ready, listening");
    axum::serve(listener, router).await.expect("server error");
}
