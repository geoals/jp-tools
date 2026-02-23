use super::*;
use crate::app::{AppState, build_router};
use crate::models::TranscriptSegment;
use crate::services::download::MockAudioDownloader;
use crate::services::export::MockAnkiExporter;
use crate::services::media::MockMediaExtractor;
use crate::services::tokenize::MockTokenizer;
use crate::services::transcribe::MockTranscriber;

use crate::services::tokenize::Token;

/// Returns a mock tokenizer that echoes each char as a single noun token.
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

async fn test_app(
    downloader: MockAudioDownloader,
    transcriber: MockTranscriber,
    exporter: MockAnkiExporter,
) -> (axum_test::TestServer, sqlx::SqlitePool) {
    test_app_with_media(downloader, transcriber, exporter, MockMediaExtractor::new()).await
}

async fn test_app_with_media(
    downloader: MockAudioDownloader,
    transcriber: MockTranscriber,
    exporter: MockAnkiExporter,
    media_extractor: MockMediaExtractor,
) -> (axum_test::TestServer, sqlx::SqlitePool) {
    let pool = db::create_pool("sqlite::memory:").await.unwrap();
    let state = AppState {
        db: pool.clone(),
        downloader: Arc::new(downloader),
        transcriber: Arc::new(transcriber),
        exporter: Arc::new(exporter),
        media_extractor: Arc::new(media_extractor),
        tokenizer: Arc::new(mock_tokenizer()),
        dictionaries: vec![],
        audio_dir: "/tmp".into(),
        media_dir: "/tmp/media".into(),
    };
    let router = build_router(state);
    let server = axum_test::TestServer::new(router).unwrap();
    (server, pool)
}

#[test]
fn format_seconds_formats_correctly() {
    assert_eq!(format_seconds(0.0), "0:00");
    assert_eq!(format_seconds(5.5), "0:05");
    assert_eq!(format_seconds(65.0), "1:05");
    assert_eq!(format_seconds(3661.0), "61:01");
}

#[tokio::test]
async fn get_mining_returns_submit_form() {
    let (server, _pool) = test_app(
        MockAudioDownloader::new(),
        MockTranscriber::new(),
        MockAnkiExporter::new(),
    )
    .await;

    let response = server.get("/mining").await;
    response.assert_status_ok();
    let body = response.text();
    assert!(body.contains("<form"), "should contain a form");
    assert!(body.contains("/mining/youtube"), "form should post to /mining/youtube");
}

#[tokio::test]
async fn post_youtube_creates_job_and_redirects() {
    let mut downloader = MockAudioDownloader::new();
    // The background task will call download, but we don't care about
    // the result for this test — just that the redirect happens.
    downloader.expect_download().returning(|_, _| {
        Box::pin(async {
            Ok(crate::services::download::DownloadResult {
                audio_path: "/tmp/audio.wav".into(),
                video_path: "/tmp/video.mp4".into(),
                video_title: "Test".into(),
            })
        })
    });

    let mut transcriber = MockTranscriber::new();
    transcriber.expect_transcribe().returning(|_| {
        Box::pin(async { Ok(vec![]) })
    });

    let (server, pool) = test_app(downloader, transcriber, MockAnkiExporter::new()).await;

    let response = server
        .post("/mining/youtube")
        .form(&SubmitForm {
            url: "https://youtube.com/watch?v=abc".into(),
        })
        .await;

    // Should redirect (303) to job page
    response.assert_status(axum::http::StatusCode::SEE_OTHER);
    let location = response.header("location").to_str().unwrap().to_string();
    assert!(location.starts_with("/mining/jobs/"));

    // Job should exist in DB
    let job = db::get_job(&pool, 1).await.unwrap();
    assert!(job.is_some());
}

#[tokio::test]
async fn job_page_pending_job_shows_status() {
    let (server, pool) = test_app(
        MockAudioDownloader::new(),
        MockTranscriber::new(),
        MockAnkiExporter::new(),
    )
    .await;

    let job_id = db::create_job(&pool, "https://youtube.com/watch?v=abc")
        .await
        .unwrap();

    let response = server.get(&format!("/mining/jobs/{job_id}")).await;
    response.assert_status_ok();
    let body = response.text();
    assert!(body.contains("pending"), "should show pending status");
    // Should NOT contain export form since job isn't done
    assert!(!body.contains("Export to Anki"));
}

#[tokio::test]
async fn job_page_done_job_shows_sentences() {
    let (server, pool) = test_app(
        MockAudioDownloader::new(),
        MockTranscriber::new(),
        MockAnkiExporter::new(),
    )
    .await;

    let job_id = db::create_job(&pool, "https://youtube.com/watch?v=abc")
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
            text: "テスト文".into(),
        }],
    )
    .await
    .unwrap();

    let response = server.get(&format!("/mining/jobs/{job_id}")).await;
    response.assert_status_ok();
    let body = response.text();
    // Sentence text is rendered as individual token spans
    assert!(body.contains("content-word"), "should show content-word token spans");
    assert!(body.contains("テ"), "should show tokenized characters");
    assert!(body.contains("Export to Anki"), "should show export button");
}

#[tokio::test]
async fn job_page_not_found() {
    let (server, _pool) = test_app(
        MockAudioDownloader::new(),
        MockTranscriber::new(),
        MockAnkiExporter::new(),
    )
    .await;

    let response = server.get("/mining/jobs/999").await;
    response.assert_status(axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn status_fragment_non_terminal_includes_polling() {
    let (server, pool) = test_app(
        MockAudioDownloader::new(),
        MockTranscriber::new(),
        MockAnkiExporter::new(),
    )
    .await;

    let job_id = db::create_job(&pool, "https://youtube.com/watch?v=abc")
        .await
        .unwrap();
    db::update_job_status(&pool, job_id, &JobStatus::Downloading, None)
        .await
        .unwrap();

    let response = server
        .get(&format!("/mining/jobs/{job_id}/status"))
        .await;
    response.assert_status_ok();
    let body = response.text();
    assert!(body.contains("hx-get"), "should include htmx polling");
}

#[tokio::test]
async fn status_fragment_terminal_redirects() {
    let (server, pool) = test_app(
        MockAudioDownloader::new(),
        MockTranscriber::new(),
        MockAnkiExporter::new(),
    )
    .await;

    let job_id = db::create_job(&pool, "https://youtube.com/watch?v=abc")
        .await
        .unwrap();
    db::update_job_status(&pool, job_id, &JobStatus::Done, None)
        .await
        .unwrap();

    let response = server
        .get(&format!("/mining/jobs/{job_id}/status"))
        .await;
    response.assert_status(axum::http::StatusCode::SEE_OTHER);
}

async fn test_app_with_media_dir(
    downloader: MockAudioDownloader,
    transcriber: MockTranscriber,
    exporter: MockAnkiExporter,
    media_extractor: MockMediaExtractor,
    media_dir: String,
) -> (axum_test::TestServer, sqlx::SqlitePool) {
    let pool = db::create_pool("sqlite::memory:").await.unwrap();
    let state = AppState {
        db: pool.clone(),
        downloader: Arc::new(downloader),
        transcriber: Arc::new(transcriber),
        exporter: Arc::new(exporter),
        media_extractor: Arc::new(media_extractor),
        tokenizer: Arc::new(mock_tokenizer()),
        dictionaries: vec![],
        audio_dir: "/tmp".into(),
        media_dir,
    };
    let router = build_router(state);
    let server = axum_test::TestServer::new(router).unwrap();
    (server, pool)
}

#[tokio::test]
async fn sentence_audio_missing_sentence_returns_404() {
    let (server, pool) = test_app(
        MockAudioDownloader::new(),
        MockTranscriber::new(),
        MockAnkiExporter::new(),
    )
    .await;

    let job_id = db::create_job(&pool, "https://youtube.com/watch?v=abc")
        .await
        .unwrap();

    let response = server
        .get(&format!("/mining/jobs/{job_id}/sentences/999/audio"))
        .await;
    response.assert_status(axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn sentence_audio_wrong_job_returns_404() {
    let (server, pool) = test_app(
        MockAudioDownloader::new(),
        MockTranscriber::new(),
        MockAnkiExporter::new(),
    )
    .await;

    let job_id = db::create_job(&pool, "https://youtube.com/watch?v=abc")
        .await
        .unwrap();
    db::insert_sentences(
        &pool,
        job_id,
        &[TranscriptSegment {
            start: 0.0,
            end: 1.0,
            text: "test".into(),
        }],
    )
    .await
    .unwrap();

    let sentences = db::get_sentences_for_job(&pool, job_id).await.unwrap();

    // Use a different job_id in the URL
    let response = server
        .get(&format!(
            "/mining/jobs/{}/sentences/{}/audio",
            job_id + 100,
            sentences[0].id
        ))
        .await;
    response.assert_status(axum::http::StatusCode::NOT_FOUND);
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

    let (server, pool) = test_app_with_media_dir(
        MockAudioDownloader::new(),
        MockTranscriber::new(),
        MockAnkiExporter::new(),
        media_extractor,
        media_dir,
    )
    .await;

    let job_id = db::create_job(&pool, "https://youtube.com/watch?v=abc")
        .await
        .unwrap();
    db::update_job_status(&pool, job_id, &JobStatus::Done, None)
        .await
        .unwrap();
    db::update_job_download(&pool, job_id, "/tmp/audio.wav", "Test", "/tmp/video.mp4")
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
        .get(&format!(
            "/mining/jobs/{job_id}/sentences/{sentence_id}/audio"
        ))
        .await;

    response.assert_status_ok();
    assert_eq!(
        response.header("content-type").to_str().unwrap(),
        "audio/mpeg"
    );
    assert_eq!(&response.as_bytes()[..], b"fake-mp3-data");
}

#[tokio::test]
async fn sentence_audio_serves_cached_clip_without_re_extracting() {
    let tmp = tempfile::tempdir().unwrap();
    let media_dir = tmp.path().to_str().unwrap().to_string();

    // Media extractor should NOT be called — the file already exists
    let media_extractor = MockMediaExtractor::new();

    let (server, pool) = test_app_with_media_dir(
        MockAudioDownloader::new(),
        MockTranscriber::new(),
        MockAnkiExporter::new(),
        media_extractor,
        media_dir.clone(),
    )
    .await;

    let job_id = db::create_job(&pool, "https://youtube.com/watch?v=abc")
        .await
        .unwrap();
    db::update_job_status(&pool, job_id, &JobStatus::Done, None)
        .await
        .unwrap();
    db::update_job_download(&pool, job_id, "/tmp/audio.wav", "Test", "/tmp/video.mp4")
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

    // Pre-create the cached clip file
    let (_, audio_filename) = media_filenames(job_id, sentence_id);
    let clip_path = format!("{media_dir}/{audio_filename}");
    std::fs::write(&clip_path, b"cached-audio").unwrap();

    let response = server
        .get(&format!(
            "/mining/jobs/{job_id}/sentences/{sentence_id}/audio"
        ))
        .await;

    response.assert_status_ok();
    assert_eq!(&response.as_bytes()[..], b"cached-audio");
}

#[tokio::test]
async fn export_calls_exporter_and_shows_success() {
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

    let (server, pool) = test_app_with_media(
        MockAudioDownloader::new(),
        MockTranscriber::new(),
        exporter,
        media_extractor,
    )
    .await;

    let job_id = db::create_job(&pool, "https://youtube.com/watch?v=abc")
        .await
        .unwrap();
    db::update_job_status(&pool, job_id, &JobStatus::Done, None)
        .await
        .unwrap();
    db::update_job_download(&pool, job_id, "/tmp/audio.wav", "Test Video", "/tmp/video.mp4")
        .await
        .unwrap();
    db::insert_sentences(
        &pool,
        job_id,
        &[
            TranscriptSegment { start: 0.0, end: 1.0, text: "A".into() },
            TranscriptSegment { start: 1.0, end: 2.0, text: "B".into() },
        ],
    )
    .await
    .unwrap();

    let all = db::get_sentences_for_job(&pool, job_id).await.unwrap();

    let form_body = format!(
        "job_id={}&sentence_ids={}",
        job_id, all[0].id
    );
    let response = server
        .post("/mining/export")
        .content_type("application/x-www-form-urlencoded")
        .bytes(form_body.into_bytes().into())
        .await;

    response.assert_status_ok();
    let body = response.text();
    assert!(body.contains("1 sentence(s) exported"));
}

#[test]
fn export_form_parses_target_words() {
    let input = "job_id=1&sentence_ids=1&target_words=1%3Ahello";
    let form: ExportForm = serde_html_form::from_str(input).unwrap();
    let map = form.target_word_map();
    assert_eq!(map.get(&1), Some(&"hello".to_string()));
}

#[test]
fn target_word_map_skips_empty_words() {
    let form = ExportForm {
        job_id: 1,
        sentence_ids: vec![1],
        target_words: vec!["1:".into(), "2:word".into()],
    };
    let map = form.target_word_map();
    assert_eq!(map.get(&1), None);
    assert_eq!(map.get(&2), Some(&"word".to_string()));
}

#[tokio::test]
async fn export_with_target_word_and_dictionary_populates_vocab() {
    use crate::services::dictionary::{Dictionary, DictionaryEntry};
    use crate::services::export::ExportSentence;

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
        .withf(|sentences: &Vec<ExportSentence>, _source: &String| {
            let s = &sentences[0];
            s.target_word.as_deref() == Some("食べる")
                && s.definition.as_deref()
                    == Some(r#"<div class="dict-unknown-title">Unknown</div><div class="dict-unknown-body">to eat; to consume</div>"#)
        })
        .returning(|sentences, _source| {
            let count = sentences.len();
            Box::pin(async move { Ok(count) })
        });

    let dict = Dictionary::from_entries(vec![DictionaryEntry {
        term: "食べる".into(),
        reading: "たべる".into(),
        definitions: vec!["to eat".into(), "to consume".into()],
        score: 100,
    }]);

    let pool = db::create_pool("sqlite::memory:").await.unwrap();
    let state = AppState {
        db: pool.clone(),
        downloader: Arc::new(MockAudioDownloader::new()),
        transcriber: Arc::new(MockTranscriber::new()),
        exporter: Arc::new(exporter),
        media_extractor: Arc::new(media_extractor),
        tokenizer: Arc::new(mock_tokenizer()),
        dictionaries: vec![Arc::new(dict)],
        audio_dir: "/tmp".into(),
        media_dir: "/tmp/media".into(),
    };
    let router = build_router(state);
    let server = axum_test::TestServer::new(router).unwrap();

    let job_id = db::create_job(&pool, "https://youtube.com/watch?v=abc")
        .await
        .unwrap();
    db::update_job_status(&pool, job_id, &JobStatus::Done, None)
        .await
        .unwrap();
    db::update_job_download(&pool, job_id, "/tmp/audio.wav", "Test Video", "/tmp/video.mp4")
        .await
        .unwrap();
    db::insert_sentences(
        &pool,
        job_id,
        &[TranscriptSegment {
            start: 0.0,
            end: 1.0,
            text: "食べる".into(),
        }],
    )
    .await
    .unwrap();

    let all = db::get_sentences_for_job(&pool, job_id).await.unwrap();
    let sid = all[0].id;

    // Submit with a target word selected: "sid:食べる" URL-encoded
    let form_body = format!(
        "job_id={job_id}&sentence_ids={sid}&target_words={sid}%3A%E9%A3%9F%E3%81%B9%E3%82%8B"
    );
    let response = server
        .post("/mining/export")
        .content_type("application/x-www-form-urlencoded")
        .bytes(form_body.into_bytes().into())
        .await;

    response.assert_status_ok();
    let body = response.text();
    assert!(body.contains("1 sentence(s) exported"));
}
