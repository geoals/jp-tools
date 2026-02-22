use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;

use tracing::warn;

use crate::app::AppState;
use crate::db;
use crate::error::AppError;
use crate::models::JobStatus;
use crate::services::export::ExportSentence;
use crate::services::media::media_filenames;
use crate::services::pipeline;

/// Form extractor that uses `serde_html_form` to handle repeated keys
/// (e.g. checkboxes: `sentence_ids=1&sentence_ids=2` -> `Vec<i64>`).
/// Standard `axum::Form` uses `serde_urlencoded` which doesn't support this.
pub(crate) struct HtmlForm<T>(T);

impl<S, T> axum::extract::FromRequest<S> for HtmlForm<T>
where
    S: Send + Sync,
    T: serde::de::DeserializeOwned,
{
    type Rejection = AppError;

    async fn from_request(
        req: axum::extract::Request,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(|e| AppError::BadRequest(e.to_string()))?;
        let value = serde_html_form::from_bytes(&bytes)
            .map_err(|e| AppError::BadRequest(e.to_string()))?;
        Ok(HtmlForm(value))
    }
}

// --- Template structs ---
// askama::Template does the rendering, askama_web::WebTemplate adds IntoResponse.

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "mining/submit.html")]
struct SubmitTemplate;

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "mining/job_status.html")]
struct JobPageTemplate {
    job_id: i64,
    video_title: Option<String>,
    status: String,
    is_done: bool,
    is_terminal: bool,
    error_message: Option<String>,
    sentences: Vec<SentenceView>,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "mining/job_status_fragment.html")]
struct JobStatusFragmentTemplate {
    job_id: i64,
    status: String,
    is_done: bool,
    is_terminal: bool,
    error_message: Option<String>,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "mining/export_success.html")]
struct ExportSuccessTemplate {
    count: usize,
}

struct SentenceView {
    id: i64,
    text: String,
    timestamp: String,
}

// --- Form structs ---

#[derive(serde::Deserialize, serde::Serialize)]
pub struct SubmitForm {
    url: String,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct ExportForm {
    job_id: i64,
    #[serde(default)]
    sentence_ids: Vec<i64>,
}

// --- Handlers ---

pub async fn submit_page() -> impl IntoResponse {
    SubmitTemplate
}

pub async fn submit_youtube(
    State(state): State<AppState>,
    Form(form): Form<SubmitForm>,
) -> Result<Response, AppError> {
    use crate::services::download::is_valid_youtube_url;

    let url = form.url.trim().to_string();
    if url.is_empty() {
        return Err(AppError::BadRequest("URL is required".into()));
    }
    if !is_valid_youtube_url(&url) {
        return Err(AppError::BadRequest("not a valid YouTube URL".into()));
    }

    let job_id = db::create_job(&state.db, &url).await?;

    let pool = state.db.clone();
    let downloader = Arc::clone(&state.downloader);
    let transcriber = Arc::clone(&state.transcriber);
    let audio_dir = state.audio_dir.clone();

    tokio::spawn(async move {
        pipeline::process_job(pool, job_id, url, audio_dir, downloader, transcriber).await;
    });

    Ok(Redirect::to(&format!("/mining/jobs/{job_id}")).into_response())
}

pub async fn job_page(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let job = db::get_job(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;

    let sentences = if job.status == JobStatus::Done {
        db::get_sentences_for_job(&state.db, id).await?
    } else {
        vec![]
    };

    let template = JobPageTemplate {
        job_id: job.id,
        video_title: job.video_title,
        status: job.status.as_str().to_string(),
        is_done: job.status == JobStatus::Done,
        is_terminal: job.status.is_terminal(),
        error_message: job.error_message,
        sentences: sentences
            .into_iter()
            .map(|s| SentenceView {
                id: s.id,
                text: s.text,
                timestamp: format_seconds(s.start_time),
            })
            .collect(),
    };

    Ok(template.into_response())
}

pub async fn job_status_fragment(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let job = db::get_job(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;

    // When terminal, redirect to the full page so htmx replaces the
    // fragment with a full page load showing sentences or error.
    if job.status.is_terminal() {
        return Ok(Redirect::to(&format!("/mining/jobs/{id}")).into_response());
    }

    let template = JobStatusFragmentTemplate {
        job_id: job.id,
        status: job.status.as_str().to_string(),
        is_done: job.status == JobStatus::Done,
        is_terminal: job.status.is_terminal(),
        error_message: job.error_message,
    };

    Ok(template.into_response())
}

pub async fn export_sentences(
    State(state): State<AppState>,
    HtmlForm(form): HtmlForm<ExportForm>,
) -> Result<Response, AppError> {
    if form.sentence_ids.is_empty() {
        return Err(AppError::BadRequest("no sentences selected".into()));
    }

    let sentences = db::get_sentences_by_ids(&state.db, &form.sentence_ids).await?;
    if sentences.is_empty() {
        return Err(AppError::NotFound);
    }

    let job = db::get_job(&state.db, form.job_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let source = job
        .video_title
        .unwrap_or_else(|| job.youtube_url.clone());

    // Extract media for each sentence (graceful degradation on failure)
    let mut export_sentences = Vec::with_capacity(sentences.len());
    for sentence in sentences {
        let (screenshot_filename, audio_filename) =
            media_filenames(sentence.job_id, sentence.id);
        let screenshot_path = format!("{}/{screenshot_filename}", state.media_dir);
        let audio_clip_path = format!("{}/{audio_filename}", state.media_dir);

        let mut screenshot_result = None;
        if let Some(video_path) = &job.video_path {
            // Use midpoint of the sentence for the screenshot
            let midpoint = (sentence.start_time + sentence.end_time) / 2.0;
            match state
                .media_extractor
                .extract_screenshot(video_path, midpoint, &screenshot_path)
                .await
            {
                Ok(()) => screenshot_result = Some(screenshot_path),
                Err(e) => warn!(
                    sentence_id = sentence.id,
                    error = %e,
                    "screenshot extraction failed, exporting without image"
                ),
            }
        }

        let mut audio_result = None;
        if let Some(audio_path) = &job.audio_path {
            match state
                .media_extractor
                .extract_audio_clip(
                    audio_path,
                    sentence.start_time,
                    sentence.end_time,
                    &audio_clip_path,
                )
                .await
            {
                Ok(()) => audio_result = Some(audio_clip_path),
                Err(e) => warn!(
                    sentence_id = sentence.id,
                    error = %e,
                    "audio clip extraction failed, exporting without audio"
                ),
            }
        }

        export_sentences.push(ExportSentence {
            sentence,
            screenshot_path: screenshot_result,
            audio_clip_path: audio_result,
        });
    }

    let count = state
        .exporter
        .export_sentences(export_sentences, source)
        .await
        .map_err(|e| AppError::Export(e.to_string()))?;

    Ok(ExportSuccessTemplate { count }.into_response())
}

fn format_seconds(secs: f64) -> String {
    let total = secs as u64;
    let minutes = total / 60;
    let seconds = total % 60;
    format!("{minutes}:{seconds:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppState, build_router};
    use crate::models::TranscriptSegment;
    use crate::services::download::MockAudioDownloader;
    use crate::services::export::MockAnkiExporter;
    use crate::services::media::MockMediaExtractor;
    use crate::services::transcribe::MockTranscriber;

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
        assert!(body.contains("テスト文"), "should show sentence text");
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
}
