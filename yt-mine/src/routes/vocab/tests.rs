use std::sync::Arc;

use axum::http::StatusCode;

use jp_core::tokenize::{MockTokenizer, Token};

use crate::app::{AppState, build_router};
use crate::db::{self, VocabUpsert};
use crate::models::VocabStatus;
use crate::services::download::MockAudioDownloader;
use crate::services::export::MockAnkiExporter;
use crate::services::media::MockMediaExtractor;
use crate::services::transcribe::MockTranscriber;

fn japanese_tokenizer() -> MockTokenizer {
    let mut t = MockTokenizer::new();
    t.expect_tokenize().returning(|text| match text {
        // base_form_reading lookup for verb conjugation grouping
        "行く" => Ok(vec![Token {
            surface: "行く".into(),
            base_form: "行く".into(),
            reading: "イク".into(),
            pos: "動詞".into(),
        }]),
        _ => Ok(vec![
            Token {
                surface: "東京".into(),
                base_form: "東京".into(),
                reading: "トウキョウ".into(),
                pos: "名詞".into(),
            },
            Token {
                surface: "に".into(),
                base_form: "に".into(),
                reading: "ニ".into(),
                pos: "助詞".into(),
            },
            Token {
                surface: "行く".into(),
                base_form: "行く".into(),
                reading: "イク".into(),
                pos: "動詞".into(),
            },
            // Duplicate 東京 to test deduplication + counting
            Token {
                surface: "東京".into(),
                base_form: "東京".into(),
                reading: "トウキョウ".into(),
                pos: "名詞".into(),
            },
        ]),
    });
    t
}

async fn test_app_with_tokenizer(
    tokenizer: MockTokenizer,
) -> (axum_test::TestServer, sqlx::SqlitePool) {
    let pool = db::create_pool("sqlite::memory:").await.unwrap();
    let state = AppState {
        db: pool.clone(),
        downloader: Arc::new(MockAudioDownloader::new()),
        transcriber: Arc::new(MockTranscriber::new()),
        exporter: Arc::new(MockAnkiExporter::new()),
        media_extractor: Arc::new(MockMediaExtractor::new()),
        tokenizer: Arc::new(tokenizer),
        dictionaries: vec![],
        llm_definer: None,
        audio_dir: "/tmp".into(),
        media_dir: "/tmp/media".into(),
    };
    let router = build_router(state);
    let server = axum_test::TestServer::new(router).unwrap();
    (server, pool)
}

// --- POST /api/vocab/tokenize ---

#[tokio::test]
async fn tokenize_returns_deduplicated_content_words() {
    let (server, _pool) = test_app_with_tokenizer(japanese_tokenizer()).await;

    let response = server
        .post("/api/vocab/tokenize")
        .json(&serde_json::json!({ "text": "東京に行く東京" }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let tokens = body["tokens"].as_array().unwrap();

    // Should have 2 content words: 東京 (count=2) and 行く (count=1)
    assert_eq!(tokens.len(), 2);

    let tokyo = tokens.iter().find(|t| t["lemma"] == "東京").unwrap();
    assert_eq!(tokyo["count"], 2);
    assert_eq!(tokyo["reading"], "トウキョウ");
    assert_eq!(tokyo["pos"], "名詞");
    assert!(tokyo["status"].is_null());
    assert_eq!(tokyo["in_db"], false);

    let iku = tokens.iter().find(|t| t["lemma"] == "行く").unwrap();
    assert_eq!(iku["count"], 1);
}

#[tokio::test]
async fn tokenize_annotates_existing_db_entries() {
    let (server, pool) = test_app_with_tokenizer(japanese_tokenizer()).await;

    // Pre-populate DB
    db::upsert_vocab_entries(
        &pool,
        &[VocabUpsert {
            user_id: 1,
            lemma: "東京".into(),
            reading: "トウキョウ".into(),
            pos: Some("名詞".into()),
            status: VocabStatus::Known,
            count: 5,
            source: "test".into(),
        }],
    )
    .await
    .unwrap();

    let response = server
        .post("/api/vocab/tokenize")
        .json(&serde_json::json!({ "text": "東京に行く東京" }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let tokens = body["tokens"].as_array().unwrap();

    let tokyo = tokens.iter().find(|t| t["lemma"] == "東京").unwrap();
    assert_eq!(tokyo["status"], "known");
    assert_eq!(tokyo["in_db"], true);
}

#[tokio::test]
async fn tokenize_filters_blacklisted_words() {
    let (server, pool) = test_app_with_tokenizer(japanese_tokenizer()).await;

    // Blacklist 東京
    db::upsert_vocab_entries(
        &pool,
        &[VocabUpsert {
            user_id: 1,
            lemma: "東京".into(),
            reading: "トウキョウ".into(),
            pos: Some("名詞".into()),
            status: VocabStatus::Blacklisted,
            count: 1,
            source: "test".into(),
        }],
    )
    .await
    .unwrap();

    let response = server
        .post("/api/vocab/tokenize")
        .json(&serde_json::json!({ "text": "東京に行く東京" }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let tokens = body["tokens"].as_array().unwrap();

    // 東京 should be filtered out, only 行く remains
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0]["lemma"], "行く");
}

#[tokio::test]
async fn tokenize_empty_text_returns_400() {
    let (server, _pool) = test_app_with_tokenizer(japanese_tokenizer()).await;

    let response = server
        .post("/api/vocab/tokenize")
        .json(&serde_json::json!({ "text": "" }))
        .await;

    response.assert_status(StatusCode::BAD_REQUEST);
}

// --- POST /api/vocab/submit ---

#[tokio::test]
async fn submit_upserts_entries() {
    let mut t = MockTokenizer::new();
    t.expect_tokenize().never();
    let (server, pool) = test_app_with_tokenizer(t).await;

    let response = server
        .post("/api/vocab/submit")
        .json(&serde_json::json!({
            "entries": [
                {
                    "lemma": "東京",
                    "reading": "トウキョウ",
                    "pos": "名詞",
                    "status": "known",
                    "count": 2
                },
                {
                    "lemma": "行く",
                    "reading": "イク",
                    "pos": "動詞",
                    "status": "seen",
                    "count": 1
                }
            ]
        }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["count"], 2);

    // Verify DB state
    let rows = db::get_vocab_by_lemmas(&pool, 1, &["東京".into(), "行く".into()])
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
}

#[tokio::test]
async fn submit_empty_entries_returns_400() {
    let mut t = MockTokenizer::new();
    t.expect_tokenize().never();
    let (server, _pool) = test_app_with_tokenizer(t).await;

    let response = server
        .post("/api/vocab/submit")
        .json(&serde_json::json!({ "entries": [] }))
        .await;

    response.assert_status(StatusCode::BAD_REQUEST);
}

// --- Conjugation grouping ---

#[tokio::test]
async fn tokenize_groups_verb_conjugations() {
    let mut t = MockTokenizer::new();
    t.expect_tokenize().returning(|text| match text {
        // base_form_reading lookup: re-tokenize "行く" → dictionary reading
        "行く" => Ok(vec![Token {
            surface: "行く".into(),
            base_form: "行く".into(),
            reading: "イク".into(),
            pos: "動詞".into(),
        }]),
        _ => Ok(vec![
            Token {
                surface: "行った".into(),
                base_form: "行く".into(),
                reading: "イッタ".into(),
                pos: "動詞".into(),
            },
            Token {
                surface: "行ける".into(),
                base_form: "行く".into(),
                reading: "イケル".into(),
                pos: "動詞".into(),
            },
        ]),
    });
    let (server, _pool) = test_app_with_tokenizer(t).await;

    let response = server
        .post("/api/vocab/tokenize")
        .json(&serde_json::json!({ "text": "行った行ける" }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let tokens = body["tokens"].as_array().unwrap();

    // Both conjugations should merge into a single 行く entry
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0]["lemma"], "行く");
    assert_eq!(tokens[0]["reading"], "イク");
    assert_eq!(tokens[0]["count"], 2);
}

// --- Non-Japanese filtering ---

#[tokio::test]
async fn tokenize_filters_non_japanese_tokens() {
    let mut t = MockTokenizer::new();
    t.expect_tokenize().returning(|_text| {
        Ok(vec![
            Token {
                surface: "100".into(),
                base_form: "100".into(),
                reading: "ヒャク".into(),
                pos: "名詞".into(),
            },
            Token {
                surface: "%".into(),
                base_form: "%".into(),
                reading: "パーセント".into(),
                pos: "名詞".into(),
            },
            Token {
                surface: "東京".into(),
                base_form: "東京".into(),
                reading: "トウキョウ".into(),
                pos: "名詞".into(),
            },
        ])
    });
    let (server, _pool) = test_app_with_tokenizer(t).await;

    let response = server
        .post("/api/vocab/tokenize")
        .json(&serde_json::json!({ "text": "100%東京" }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let tokens = body["tokens"].as_array().unwrap();

    // "100" and "%" should be filtered out, only 東京 remains
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0]["lemma"], "東京");
}
