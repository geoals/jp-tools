use std::sync::Arc;

use axum::http::StatusCode;

use jp_core::dictionary::{Dictionary, DictionaryEntry, PitchEntry};
use jp_core::tokenize::{MockTokenizer, Token};

use crate::app::{AppState, build_router};
use crate::db;
use crate::models::{JobStatus, TranscriptSegment};
use crate::services::download::MockAudioDownloader;
use crate::services::export::MockAnkiExporter;
use crate::services::llm::MockLlmDefiner;
use crate::services::media::{MockMediaExtractor, media_filenames};
use crate::services::transcribe::MockTranscriber;


fn mock_tokenizer() -> MockTokenizer {
    let mut t = MockTokenizer::new();
    t.expect_tokenize().returning(|text| {
        Ok(text
            .chars()
            .map(|c| Token {
                surface: c.to_string(),
                base_form: c.to_string(),
                reading: "*".into(),
                pos: "名詞".into(),
            })
            .collect())
    });
    t
}

async fn test_app() -> (axum_test::TestServer, sqlx::SqlitePool) {
    let pool = db::create_pool("sqlite::memory:").await.unwrap();
    let state = AppState {
        db: pool.clone(),
        downloader: Arc::new(MockAudioDownloader::new()),
        transcriber: Arc::new(MockTranscriber::new()),
        exporter: Arc::new(MockAnkiExporter::new()),
        media_extractor: Arc::new(MockMediaExtractor::new()),
        tokenizer: Arc::new(mock_tokenizer()),
        dictionaries: vec![],
        llm_definer: None,
        audio_dir: "/tmp".into(),
        media_dir: "/tmp/media".into(),
    };
    let router = build_router(state);
    let server = axum_test::TestServer::new(router).unwrap();
    (server, pool)
}

async fn seed_job(pool: &sqlx::SqlitePool, video_id: &str) -> i64 {
    let job_id = db::create_job(pool, &format!("https://youtube.com/watch?v={video_id}"), video_id)
        .await
        .unwrap();
    db::update_job_status(pool, job_id, &JobStatus::Done, None)
        .await
        .unwrap();
    db::insert_sentences(
        pool,
        job_id,
        &[TranscriptSegment {
            start: 0.0,
            end: 3.0,
            text: "テスト文".into(),
        }],
    )
    .await
    .unwrap();
    job_id
}

// --- POST /api/jobs ---

#[tokio::test]
async fn submit_job_creates_and_returns_video_id() {
    let mut downloader = MockAudioDownloader::new();
    downloader.expect_download().returning(|_, _| {
        Box::pin(async {
            Ok(crate::services::download::DownloadResult {
                audio_path: "/tmp/audio.wav".into(),
                video_path: "/tmp/video.mp4".into(),
                video_title: "Test".into(),
                video_duration: Some(60.0),
            })
        })
    });

    let mut transcriber = MockTranscriber::new();
    transcriber
        .expect_transcribe()
        .returning(|_, _| Box::pin(async { Ok(vec![]) }));

    let pool = db::create_pool("sqlite::memory:").await.unwrap();
    let state = AppState {
        db: pool.clone(),
        downloader: Arc::new(downloader),
        transcriber: Arc::new(transcriber),
        exporter: Arc::new(MockAnkiExporter::new()),
        media_extractor: Arc::new(MockMediaExtractor::new()),
        tokenizer: Arc::new(mock_tokenizer()),
        dictionaries: vec![],
        llm_definer: None,
        audio_dir: "/tmp".into(),
        media_dir: "/tmp/media".into(),
    };
    let router = build_router(state);
    let server = axum_test::TestServer::new(router).unwrap();

    let response = server
        .post("/api/jobs")
        .json(&serde_json::json!({ "url": "https://youtube.com/watch?v=dQw4w9WgXcQ" }))
        .await;

    response.assert_status(StatusCode::CREATED);
    let body: serde_json::Value = response.json();
    assert_eq!(body["video_id"], "dQw4w9WgXcQ");
}

#[tokio::test]
async fn submit_job_invalid_url_returns_400() {
    let (server, _pool) = test_app().await;

    let response = server
        .post("/api/jobs")
        .json(&serde_json::json!({ "url": "not-a-url" }))
        .await;

    response.assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn submit_job_existing_returns_200() {
    let (server, pool) = test_app().await;
    seed_job(&pool, "dQw4w9WgXcQ").await;

    let response = server
        .post("/api/jobs")
        .json(&serde_json::json!({ "url": "https://youtube.com/watch?v=dQw4w9WgXcQ" }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["video_id"], "dQw4w9WgXcQ");
}

// --- GET /api/{video_id} ---

#[tokio::test]
async fn get_job_returns_sentences_and_tokens() {
    let (server, pool) = test_app().await;
    seed_job(&pool, "dQw4w9WgXcQ").await;

    let response = server.get("/api/dQw4w9WgXcQ").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();

    assert_eq!(body["status"], "done");
    assert_eq!(body["is_terminal"], true);
    assert_eq!(body["sentence_count"], 1);

    let sentences = body["sentences"].as_array().unwrap();
    assert_eq!(sentences.len(), 1);
    assert_eq!(sentences[0]["text"], "テスト文");

    let tokens = sentences[0]["tokens"].as_array().unwrap();
    assert!(!tokens.is_empty());
    assert!(tokens[0]["is_content_word"].as_bool().unwrap());
}

#[tokio::test]
async fn get_job_not_found_returns_404() {
    let (server, _pool) = test_app().await;

    let response = server.get("/api/nonexistent1").await;
    response.assert_status(StatusCode::NOT_FOUND);
}

// --- GET /api/{video_id}/status ---

#[tokio::test]
async fn poll_status_returns_job_data() {
    let (server, pool) = test_app().await;
    seed_job(&pool, "dQw4w9WgXcQ").await;

    let response = server.get("/api/dQw4w9WgXcQ/status").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["status"], "done");
}

#[tokio::test]
async fn poll_status_returns_204_when_unchanged() {
    let (server, pool) = test_app().await;
    seed_job(&pool, "dQw4w9WgXcQ").await;

    let response = server
        .get("/api/dQw4w9WgXcQ/status?sc=1&st=done")
        .await;
    response.assert_status(StatusCode::NO_CONTENT);
}

// --- GET /api/{video_id}/sentences/{id}/preview ---

#[tokio::test]
async fn preview_returns_json() {
    let mut dict = Dictionary::from_entries(vec![DictionaryEntry {
        term: "食べる".into(),
        reading: "たべる".into(),
        definitions: vec!["to eat".into()],
        score: 100,
    }]);
    dict.set_pitch(vec![(
        "食べる".into(),
        PitchEntry {
            reading: "たべる".into(),
            positions: vec![2],
        },
    )]);

    let pool = db::create_pool("sqlite::memory:").await.unwrap();
    let state = AppState {
        db: pool.clone(),
        downloader: Arc::new(MockAudioDownloader::new()),
        transcriber: Arc::new(MockTranscriber::new()),
        exporter: Arc::new(MockAnkiExporter::new()),
        media_extractor: Arc::new(MockMediaExtractor::new()),
        tokenizer: Arc::new(mock_tokenizer()),
        dictionaries: vec![Arc::new(dict)],
        llm_definer: None,
        audio_dir: "/tmp".into(),
        media_dir: "/tmp/media".into(),
    };
    let router = build_router(state);
    let server = axum_test::TestServer::new(router).unwrap();

    let job_id = db::create_job(&pool, "https://youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ")
        .await
        .unwrap();
    db::update_job_status(&pool, job_id, &JobStatus::Done, None)
        .await
        .unwrap();
    db::insert_sentences(
        &pool,
        job_id,
        &[TranscriptSegment {
            start: 0.0,
            end: 3.0,
            text: "食べる".into(),
        }],
    )
    .await
    .unwrap();

    let sentences = db::get_sentences_for_job(&pool, job_id).await.unwrap();
    let sid = sentences[0].id;

    let response = server
        .get(&format!(
            "/api/dQw4w9WgXcQ/sentences/{sid}/preview?word=%E9%A3%9F%E3%81%B9%E3%82%8B"
        ))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["word"], "食べる");
    assert_eq!(body["reading"], "たべる");
    assert_eq!(body["pitch_num"], "2");
    assert!(body["definition_html"].as_str().unwrap().contains("to eat"));
}

// --- GET /api/{video_id}/sentences/{id}/llm-definition ---

#[tokio::test]
async fn llm_definition_returns_json() {
    let mut definer = MockLlmDefiner::new();
    definer
        .expect_define()
        .returning(|_word, _ctx| Box::pin(async { Ok("A verb meaning to eat.".into()) }));

    let pool = db::create_pool("sqlite::memory:").await.unwrap();
    let state = AppState {
        db: pool.clone(),
        downloader: Arc::new(MockAudioDownloader::new()),
        transcriber: Arc::new(MockTranscriber::new()),
        exporter: Arc::new(MockAnkiExporter::new()),
        media_extractor: Arc::new(MockMediaExtractor::new()),
        tokenizer: Arc::new(mock_tokenizer()),
        dictionaries: vec![],
        llm_definer: Some(Arc::new(definer)),
        audio_dir: "/tmp".into(),
        media_dir: "/tmp/media".into(),
    };
    let router = build_router(state);
    let server = axum_test::TestServer::new(router).unwrap();

    let job_id = db::create_job(&pool, "https://youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ")
        .await
        .unwrap();
    db::update_job_status(&pool, job_id, &JobStatus::Done, None)
        .await
        .unwrap();
    db::insert_sentences(
        &pool,
        job_id,
        &[TranscriptSegment {
            start: 0.0,
            end: 3.0,
            text: "食べる".into(),
        }],
    )
    .await
    .unwrap();

    let sentences = db::get_sentences_for_job(&pool, job_id).await.unwrap();
    let sid = sentences[0].id;

    let response = server
        .get(&format!(
            "/api/dQw4w9WgXcQ/sentences/{sid}/llm-definition?word=%E9%A3%9F%E3%81%B9%E3%82%8B"
        ))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["definition"], "A verb meaning to eat.");
}

#[tokio::test]
async fn llm_definition_without_definer_returns_null() {
    let (server, pool) = test_app().await;
    seed_job(&pool, "dQw4w9WgXcQ").await;

    let sentences = db::get_sentences_for_job(&pool, 1).await.unwrap();
    let sid = sentences[0].id;

    let response = server
        .get(&format!(
            "/api/dQw4w9WgXcQ/sentences/{sid}/llm-definition?word=%E3%83%86%E3%82%B9%E3%83%88"
        ))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert!(body["definition"].is_null());
}

// --- POST /api/export ---

#[tokio::test]
async fn export_returns_count_and_ids() {
    let mut media_extractor = MockMediaExtractor::new();
    media_extractor
        .expect_extract_screenshot()
        .returning(|_, _, _| Box::pin(async { Ok(()) }));
    media_extractor
        .expect_extract_audio_clip()
        .returning(|_, _, _, _| Box::pin(async { Ok(()) }));

    let mut exporter = MockAnkiExporter::new();
    exporter
        .expect_export_sentences()
        .returning(|sentences, _source| {
            let count = sentences.len();
            Box::pin(async move { Ok(count) })
        });

    let pool = db::create_pool("sqlite::memory:").await.unwrap();
    let state = AppState {
        db: pool.clone(),
        downloader: Arc::new(MockAudioDownloader::new()),
        transcriber: Arc::new(MockTranscriber::new()),
        exporter: Arc::new(exporter),
        media_extractor: Arc::new(media_extractor),
        tokenizer: Arc::new(mock_tokenizer()),
        dictionaries: vec![],
        llm_definer: None,
        audio_dir: "/tmp".into(),
        media_dir: "/tmp/media".into(),
    };
    let router = build_router(state);
    let server = axum_test::TestServer::new(router).unwrap();

    let job_id = db::create_job(&pool, "https://youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ")
        .await
        .unwrap();
    db::update_job_status(&pool, job_id, &JobStatus::Done, None)
        .await
        .unwrap();
    db::update_job_download(&pool, job_id, "/tmp/audio.wav", "Test", "/tmp/video.mp4", Some(60.0))
        .await
        .unwrap();
    db::insert_sentences(
        &pool,
        job_id,
        &[TranscriptSegment {
            start: 0.0,
            end: 1.0,
            text: "テスト".into(),
        }],
    )
    .await
    .unwrap();

    let sentences = db::get_sentences_for_job(&pool, job_id).await.unwrap();
    let sid = sentences[0].id;

    let response = server
        .post("/api/export")
        .json(&serde_json::json!({
            "job_id": job_id,
            "sentences": [{ "id": sid }]
        }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["count"], 1);
    assert_eq!(body["exported_ids"], serde_json::json!([sid]));
}

#[tokio::test]
async fn export_empty_returns_400() {
    let (server, _pool) = test_app().await;

    let response = server
        .post("/api/export")
        .json(&serde_json::json!({
            "job_id": 1,
            "sentences": []
        }))
        .await;

    response.assert_status(StatusCode::BAD_REQUEST);
    let body: serde_json::Value = response.json();
    assert!(body["error"].as_str().unwrap().contains("No sentences"));
}

// --- sentence audio ---

async fn test_app_with_media_dir(
    media_extractor: MockMediaExtractor,
    media_dir: String,
) -> (axum_test::TestServer, sqlx::SqlitePool) {
    let pool = db::create_pool("sqlite::memory:").await.unwrap();
    let state = AppState {
        db: pool.clone(),
        downloader: Arc::new(MockAudioDownloader::new()),
        transcriber: Arc::new(MockTranscriber::new()),
        exporter: Arc::new(MockAnkiExporter::new()),
        media_extractor: Arc::new(media_extractor),
        tokenizer: Arc::new(mock_tokenizer()),
        dictionaries: vec![],
        llm_definer: None,
        audio_dir: "/tmp".into(),
        media_dir,
    };
    let router = build_router(state);
    let server = axum_test::TestServer::new(router).unwrap();
    (server, pool)
}

#[tokio::test]
async fn sentence_audio_missing_sentence_returns_404() {
    let (server, pool) = test_app().await;

    db::create_job(&pool, "https://youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ")
        .await
        .unwrap();

    let response = server.get("/dQw4w9WgXcQ/sentences/999/audio").await;
    response.assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn sentence_audio_extracts_and_returns_mp3() {
    let tmp = tempfile::tempdir().unwrap();
    let media_dir = tmp.path().to_str().unwrap().to_string();

    let mut media_extractor = MockMediaExtractor::new();
    media_extractor
        .expect_extract_audio_clip()
        .returning(move |_, _, _, output_path: &str| {
            let path = output_path.to_owned();
            Box::pin(async move {
                tokio::fs::write(&path, b"fake-mp3-data").await.unwrap();
                Ok(())
            })
        });

    let (server, pool) = test_app_with_media_dir(media_extractor, media_dir).await;

    let job_id = db::create_job(&pool, "https://youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ")
        .await
        .unwrap();
    db::update_job_status(&pool, job_id, &JobStatus::Done, None)
        .await
        .unwrap();
    db::update_job_download(&pool, job_id, "/tmp/audio.wav", "Test", "/tmp/video.mp4", Some(60.0))
        .await
        .unwrap();
    db::insert_sentences(
        &pool,
        job_id,
        &[TranscriptSegment {
            start: 1.0,
            end: 3.0,
            text: "audio test".into(),
        }],
    )
    .await
    .unwrap();

    let sentences = db::get_sentences_for_job(&pool, job_id).await.unwrap();
    let sentence_id = sentences[0].id;

    let response = server
        .get(&format!("/dQw4w9WgXcQ/sentences/{sentence_id}/audio"))
        .await;

    response.assert_status_ok();
    assert_eq!(
        response.header("content-type").to_str().unwrap(),
        "audio/mpeg"
    );
    assert_eq!(&response.as_bytes()[..], b"fake-mp3-data");
}

#[tokio::test]
async fn sentence_audio_serves_cached_clip() {
    let tmp = tempfile::tempdir().unwrap();
    let media_dir = tmp.path().to_str().unwrap().to_string();

    // No extract expectation — the cached file should be served directly
    let media_extractor = MockMediaExtractor::new();
    let (server, pool) = test_app_with_media_dir(media_extractor, media_dir.clone()).await;

    let job_id = db::create_job(&pool, "https://youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ")
        .await
        .unwrap();
    db::update_job_status(&pool, job_id, &JobStatus::Done, None)
        .await
        .unwrap();
    db::update_job_download(&pool, job_id, "/tmp/audio.wav", "Test", "/tmp/video.mp4", Some(60.0))
        .await
        .unwrap();
    db::insert_sentences(
        &pool,
        job_id,
        &[TranscriptSegment {
            start: 0.0,
            end: 1.0,
            text: "cached".into(),
        }],
    )
    .await
    .unwrap();

    let sentences = db::get_sentences_for_job(&pool, job_id).await.unwrap();
    let sentence_id = sentences[0].id;

    // Pre-create cached clip
    let (_, audio_filename) = media_filenames(job_id, sentence_id);
    let clip_path = format!("{media_dir}/{audio_filename}");
    std::fs::write(&clip_path, b"cached-audio").unwrap();

    let response = server
        .get(&format!("/dQw4w9WgXcQ/sentences/{sentence_id}/audio"))
        .await;

    response.assert_status_ok();
    assert_eq!(&response.as_bytes()[..], b"cached-audio");
}

// --- SPA shell ---

#[tokio::test]
async fn root_returns_spa_shell() {
    let (server, _pool) = test_app().await;

    let response = server.get("/").await;
    response.assert_status_ok();
    let body = response.text();
    assert!(body.contains("<div id=\"app\">"), "should contain app mount point");
    assert!(body.contains("/static/app.js"), "should include Preact entry point");
}

#[tokio::test]
async fn video_url_returns_spa_shell() {
    let (server, _pool) = test_app().await;

    let response = server.get("/dQw4w9WgXcQ").await;
    response.assert_status_ok();
    let body = response.text();
    assert!(body.contains("<div id=\"app\">"), "should contain app mount point");
}

#[tokio::test]
async fn vocab_url_returns_spa_shell() {
    let (server, _pool) = test_app().await;

    let response = server.get("/vocab").await;
    response.assert_status_ok();
    let body = response.text();
    assert!(body.contains("<div id=\"app\">"), "should contain app mount point");
}
