use std::sync::Arc;

use tracing::info;

use jp_core::highlight::Highlighter;
use jp_core::tokenize::Tokenizer;

use yt_mine::app::{AppState, build_router};
use yt_mine::config::Config;
use yt_mine::db;
use yt_mine::services::download::{AudioDownloader, YtDlpDownloader};
use yt_mine::services::export::{AnkiConnectExporter, AnkiExporter};
use yt_mine::services::fake::{
    FakeAnkiExporter, FakeDownloader, FakeLlmDefiner, FakeMediaExtractor, FakeTokenizer,
    FakeTranscriber,
};
use yt_mine::services::llm::{CompactDefiner, LlmDefiner};
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

    let (downloader, transcriber, exporter, media_extractor, tokenizer, highlighter): (
        Arc<dyn AudioDownloader>,
        Arc<dyn Transcriber>,
        Arc<dyn AnkiExporter>,
        Arc<dyn MediaExtractor>,
        Arc<dyn Tokenizer>,
        Option<Arc<Highlighter>>,
    ) = if config.fake_api {
        info!("*** DEV MODE — using fake services (no external deps needed) ***");
        (
            Arc::new(FakeDownloader),
            Arc::new(FakeTranscriber),
            Arc::new(FakeAnkiExporter),
            Arc::new(FakeMediaExtractor),
            Arc::new(FakeTokenizer),
            None,
        )
    } else {
        // The reader's own pipeline, all seven inputs — not a bare Sudachi with
        // the headword list, which is what this used to build. A transcript and
        // a hooked VN line have to be segmented the same way, or a word mined
        // here lands on a ledger key `#read` never produces.
        let highlighter = Arc::new(
            Highlighter::build(&knowledge, &config.sudachi_dict_path)
                .await
                .expect("failed to build the tokenizer"),
        );
        info!("tokenizer ready (jp-core pipeline)");

        info!(url = %config.whisper_service_url, "using remote whisper service");
        let transcriber: Arc<dyn Transcriber> =
            Arc::new(RemoteTranscriber::new(config.whisper_service_url.clone()));

        (
            Arc::new(YtDlpDownloader),
            transcriber,
            Arc::new(AnkiConnectExporter::new(config.anki_url, config.anki)),
            Arc::new(FfmpegMediaExtractor),
            highlighter.tokenizer(),
            Some(highlighter),
        )
    };

    let llm_definer: Option<Arc<dyn LlmDefiner>> = if config.fake_api {
        Some(Arc::new(FakeLlmDefiner))
    } else {
        config.anthropic_api_key.as_ref().map(|key| {
            info!("CompactDef enabled");
            Arc::new(CompactDefiner::new(key.clone())) as Arc<dyn LlmDefiner>
        })
    };

    let state = AppState {
        db: pool,
        downloader,
        transcriber,
        exporter,
        media_extractor,
        tokenizer,
        highlighter,
        knowledge,
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
