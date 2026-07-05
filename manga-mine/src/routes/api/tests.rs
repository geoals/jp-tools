use std::sync::Arc;

use axum_test::TestServer;
use image::codecs::jpeg::JpegEncoder;
use image::{Rgb, RgbImage};

use jp_mine_core::export::MockAnkiExporter;

use crate::app::{AppState, build_router};
use crate::services::fake::{FakeAnkiExporter, FakeTokenizer};
use crate::services::ocr::MockOcrEngine;

fn test_jpeg_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut img = RgbImage::new(width, height);
    for (_, _, p) in img.enumerate_pixels_mut() {
        *p = Rgb([180, 180, 180]);
    }
    let mut out = Vec::new();
    img.write_with_encoder(JpegEncoder::new_with_quality(&mut out, 85)).unwrap();
    out
}

struct TestEnv {
    server: TestServer,
    inbox: tempfile::TempDir,
    _media: tempfile::TempDir,
}

fn build_env(ocr: MockOcrEngine, exporter: Arc<dyn jp_mine_core::export::AnkiExporter>) -> TestEnv {
    let inbox = tempfile::tempdir().unwrap();
    let media = tempfile::tempdir().unwrap();

    let state = AppState {
        tokenizer: Arc::new(FakeTokenizer),
        dictionaries: vec![],
        ocr: Arc::new(ocr),
        exporter,
        inbox_dir: inbox.path().to_path_buf(),
        media_dir: media.path().to_string_lossy().into_owned(),
        anki_config: jp_mine_core::config::AnkiConfig::default(),
        use_client_anki: false,
        card_image_max_dim: 1280,
        card_image_quality: 80,
    };

    TestEnv {
        server: TestServer::new(build_router(state)).unwrap(),
        inbox,
        _media: media,
    }
}

fn default_env() -> TestEnv {
    build_env(MockOcrEngine::new(), Arc::new(FakeAnkiExporter))
}

// --- Queue ---

#[tokio::test]
async fn queue_lists_only_image_files() {
    let env = default_env();
    std::fs::write(env.inbox.path().join("a.jpg"), test_jpeg_bytes(10, 10)).unwrap();
    std::fs::write(env.inbox.path().join("b.txt"), b"not an image").unwrap();
    std::fs::write(env.inbox.path().join(".hidden.jpg"), b"dotfile").unwrap();
    std::fs::create_dir(env.inbox.path().join("processed")).unwrap();

    let response = env.server.get("/api/queue").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let photos = body["photos"].as_array().unwrap();
    assert_eq!(photos.len(), 1);
    assert_eq!(photos[0]["name"], "a.jpg");
}

#[tokio::test]
async fn queue_empty_inbox_is_empty_list() {
    let env = default_env();
    let response = env.server.get("/api/queue").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["photos"].as_array().unwrap().len(), 0);
}

// --- Photo serving ---

#[tokio::test]
async fn get_photo_serves_bytes() {
    let env = default_env();
    let jpeg = test_jpeg_bytes(20, 20);
    std::fs::write(env.inbox.path().join("page.jpg"), &jpeg).unwrap();

    let response = env.server.get("/api/photos/page.jpg").await;
    response.assert_status_ok();
    assert_eq!(response.header("content-type"), "image/jpeg");
    assert_eq!(response.as_bytes().to_vec(), jpeg);
}

#[tokio::test]
async fn get_photo_missing_is_404() {
    let env = default_env();
    let response = env.server.get("/api/photos/nope.jpg").await;
    response.assert_status_not_found();
}

#[tokio::test]
async fn get_photo_rejects_traversal() {
    let env = default_env();
    let response = env.server.get("/api/photos/..%2Fsecret.jpg").await;
    assert!(response.status_code().is_client_error());
}

#[tokio::test]
async fn thumbnail_downscales() {
    let env = default_env();
    std::fs::write(env.inbox.path().join("big.jpg"), test_jpeg_bytes(1200, 800)).unwrap();

    let response = env.server.get("/api/photos/big.jpg/thumb").await;
    response.assert_status_ok();
    let img = image::load_from_memory(response.as_bytes()).unwrap();
    assert!(img.width() <= 480 && img.height() <= 480);
}

// --- Upload ---

#[tokio::test]
async fn upload_stores_photo_in_inbox() {
    let env = default_env();
    let jpeg = test_jpeg_bytes(10, 10);

    let response = env
        .server
        .post("/api/photos")
        .multipart(
            axum_test::multipart::MultipartForm::new().add_part(
                "photo",
                axum_test::multipart::Part::bytes(jpeg.clone())
                    .file_name("IMG_0042.jpg")
                    .mime_type("image/jpeg"),
            ),
        )
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
    let body: serde_json::Value = response.json();
    assert_eq!(body["name"], "IMG_0042.jpg");
    assert!(env.inbox.path().join("IMG_0042.jpg").exists());
}

#[tokio::test]
async fn upload_deduplicates_names() {
    let env = default_env();
    std::fs::write(env.inbox.path().join("IMG_0042.jpg"), b"existing").unwrap();

    let response = env
        .server
        .post("/api/photos")
        .multipart(
            axum_test::multipart::MultipartForm::new().add_part(
                "photo",
                axum_test::multipart::Part::bytes(test_jpeg_bytes(10, 10))
                    .file_name("IMG_0042.jpg")
                    .mime_type("image/jpeg"),
            ),
        )
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
    let body: serde_json::Value = response.json();
    assert_eq!(body["name"], "IMG_0042-1.jpg");
}

// --- OCR ---

#[tokio::test]
async fn ocr_crop_returns_tokenized_sentences() {
    let mut ocr = MockOcrEngine::new();
    ocr.expect_recognize()
        .returning(|_| Box::pin(async { Ok("お前は強い。本当？".to_string()) }));
    let env = build_env(ocr, Arc::new(FakeAnkiExporter));

    std::fs::write(env.inbox.path().join("panel.jpg"), test_jpeg_bytes(200, 200)).unwrap();

    let response = env
        .server
        .post("/api/photos/panel.jpg/ocr")
        .json(&serde_json::json!({ "x": 0.1, "y": 0.1, "w": 0.5, "h": 0.5 }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["text"], "お前は強い。本当？");
    let sentences = body["sentences"].as_array().unwrap();
    assert_eq!(sentences.len(), 2);
    assert_eq!(sentences[0]["text"], "お前は強い。");
    assert_eq!(sentences[1]["text"], "本当？");
    // FakeTokenizer is char-level; kanji are content words
    let tokens = sentences[0]["tokens"].as_array().unwrap();
    assert!(tokens.iter().any(|t| t["is_content_word"] == true));
}

#[tokio::test]
async fn ocr_empty_crop_is_client_error() {
    let env = default_env();
    std::fs::write(env.inbox.path().join("panel.jpg"), test_jpeg_bytes(50, 50)).unwrap();

    let response = env
        .server
        .post("/api/photos/panel.jpg/ocr")
        .json(&serde_json::json!({ "x": 0.5, "y": 0.5, "w": 0.0, "h": 0.0 }))
        .await;

    assert!(response.status_code().is_server_error() || response.status_code().is_client_error());
}

// --- Export ---

#[tokio::test]
async fn export_sends_card_with_compressed_image() {
    let mut exporter = MockAnkiExporter::new();
    exporter
        .expect_export_sentences()
        .withf(|sentences| {
            let es = &sentences[0];
            es.sentence_text == "お前は強い。"
                && es.target_word.as_deref() == Some("強")
                && es.source == "panel"
                && es.audio_clip_path.is_none()
                && es
                    .screenshot_path
                    .as_deref()
                    .map(|p| {
                        // compressed image was written before the export call
                        std::path::Path::new(p).exists()
                            && p.contains("manga-mine_panel_")
                    })
                    .unwrap_or(false)
                // FakeTokenizer is char-level, so 強 gets bolded
                && es.sentence_html.as_deref() == Some("お前は<b>強</b>い。")
        })
        .returning(|sentences| {
            let count = sentences.len();
            Box::pin(async move { Ok(count) })
        });

    let env = build_env(MockOcrEngine::new(), Arc::new(exporter));
    std::fs::write(env.inbox.path().join("panel.jpg"), test_jpeg_bytes(2000, 1500)).unwrap();

    let response = env
        .server
        .post("/api/export")
        .json(&serde_json::json!({
            "photo": "panel.jpg",
            "sentence": "お前は強い。",
            "target_word": "強"
        }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["count"], 1);
}

#[tokio::test]
async fn export_requires_sentence() {
    let env = default_env();
    std::fs::write(env.inbox.path().join("panel.jpg"), test_jpeg_bytes(50, 50)).unwrap();

    let response = env
        .server
        .post("/api/export")
        .json(&serde_json::json!({ "photo": "panel.jpg", "sentence": "  " }))
        .await;
    response.assert_status_bad_request();
}

// --- Mark processed/skipped ---

#[tokio::test]
async fn mark_moves_photo_out_of_queue() {
    let env = default_env();
    std::fs::write(env.inbox.path().join("done.jpg"), test_jpeg_bytes(10, 10)).unwrap();

    let response = env
        .server
        .post("/api/photos/done.jpg/mark")
        .json(&serde_json::json!({ "status": "processed" }))
        .await;
    response.assert_status(axum::http::StatusCode::NO_CONTENT);

    assert!(!env.inbox.path().join("done.jpg").exists());
    assert!(env.inbox.path().join("processed/done.jpg").exists());

    // Queue no longer includes it
    let response = env.server.get("/api/queue").await;
    let body: serde_json::Value = response.json();
    assert_eq!(body["photos"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn mark_rejects_unknown_status() {
    let env = default_env();
    std::fs::write(env.inbox.path().join("x.jpg"), test_jpeg_bytes(10, 10)).unwrap();

    let response = env
        .server
        .post("/api/photos/x.jpg/mark")
        .json(&serde_json::json!({ "status": "deleted" }))
        .await;
    response.assert_status_bad_request();
}

#[tokio::test]
async fn mark_skipped_moves_to_skipped_folder() {
    let env = default_env();
    std::fs::write(env.inbox.path().join("skip.jpg"), test_jpeg_bytes(10, 10)).unwrap();

    let response = env
        .server
        .post("/api/photos/skip.jpg/mark")
        .json(&serde_json::json!({ "status": "skipped" }))
        .await;
    response.assert_status(axum::http::StatusCode::NO_CONTENT);
    assert!(env.inbox.path().join("skipped/skip.jpg").exists());
}
