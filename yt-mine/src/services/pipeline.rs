use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::task::JoinHandle;
use tracing::{error, info};

use crate::db;
use crate::models::JobStatus;
use crate::services::download::AudioDownloader;
use crate::services::transcribe::{ProgressCallback, Transcriber};

/// Runs the full pipeline for a job: download -> transcribe -> store sentences.
/// Updates job status at each step. On failure, sets status to `error` with a message.
pub async fn process_job(
    pool: SqlitePool,
    job_id: i64,
    youtube_url: String,
    audio_dir: String,
    downloader: Arc<dyn AudioDownloader>,
    transcriber: Arc<dyn Transcriber>,
) {
    // Step 1: Download
    info!(job_id, url = youtube_url, "starting download");
    db::update_job_status(&pool, job_id, &JobStatus::Downloading, None)
        .await
        .ok();

    let download_result = match downloader
        .download(youtube_url, audio_dir)
        .await
    {
        Ok(result) => result,
        Err(e) => {
            error!(job_id, error = %e, "download failed");
            db::update_job_status(&pool, job_id, &JobStatus::Error, Some("Download failed."))
                .await
                .ok();
            return;
        }
    };

    db::update_job_download(
        &pool,
        job_id,
        &download_result.audio_path,
        &download_result.video_title,
        &download_result.video_path,
    )
    .await
    .ok();

    // Step 2: Transcribe — sentences are inserted progressively via the callback,
    // so they appear in the UI during transcription (htmx polling picks them up).
    info!(job_id, "starting transcription");
    db::update_job_status(&pool, job_id, &JobStatus::Transcribing, None)
        .await
        .ok();

    let progress_pool = pool.clone();
    let write_handles: Arc<std::sync::Mutex<Vec<JoinHandle<()>>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let cb_handles = write_handles.clone();

    let on_progress: Option<ProgressCallback> = Some(Box::new(move |segment, _count| {
        let pool = progress_pool.clone();
        let handle = tokio::spawn(async move {
            db::insert_sentence(&pool, job_id, &segment).await.ok();
        });
        cb_handles.lock().unwrap().push(handle);
    }));

    let segments = match transcriber
        .transcribe(download_result.audio_path, on_progress)
        .await
    {
        Ok(segments) => segments,
        Err(e) => {
            error!(job_id, error = %e, "transcription failed");
            db::update_job_status(&pool, job_id, &JobStatus::Error, Some("Transcription failed."))
                .await
                .ok();
            return;
        }
    };

    // Ensure all sentence inserts finish before marking the job as Done.
    let pending: Vec<_> = write_handles.lock().unwrap().drain(..).collect();
    for handle in pending {
        handle.await.ok();
    }

    info!(job_id, count = segments.len(), "job complete");
    db::update_job_status(&pool, job_id, &JobStatus::Done, None)
        .await
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TranscriptSegment;
    use crate::services::download::{DownloadError, DownloadResult, MockAudioDownloader};
    use crate::services::transcribe::{MockTranscriber, TranscribeError};

    async fn test_pool() -> SqlitePool {
        db::create_pool("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn happy_path_downloads_transcribes_and_stores() {
        let pool = test_pool().await;
        let job_id = db::create_job(&pool, "https://youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ")
            .await
            .unwrap();

        let mut downloader = MockAudioDownloader::new();
        downloader.expect_download().returning(|_, _| {
            Box::pin(async {
                Ok(DownloadResult {
                    audio_path: "/tmp/audio.wav".into(),
                    video_path: "/tmp/video.mp4".into(),
                    video_title: "Test Video".into(),
                })
            })
        });

        let mut transcriber = MockTranscriber::new();
        transcriber.expect_transcribe().returning(|_, on_progress| {
            Box::pin(async move {
                let segments = vec![
                    TranscriptSegment { start: 0.0, end: 3.0, text: "Hello".into() },
                    TranscriptSegment { start: 3.0, end: 6.0, text: "World".into() },
                ];
                if let Some(cb) = &on_progress {
                    for (i, seg) in segments.iter().enumerate() {
                        cb(seg.clone(), i + 1);
                    }
                }
                Ok(segments)
            })
        });

        process_job(
            pool.clone(),
            job_id,
            "https://youtube.com/watch?v=abc".into(),
            "/tmp".into(),
            Arc::new(downloader),
            Arc::new(transcriber),
        )
        .await;

        let job = db::get_job(&pool, job_id).await.unwrap().unwrap();
        assert_eq!(job.status, JobStatus::Done);
        assert_eq!(job.video_title.as_deref(), Some("Test Video"));
        assert_eq!(job.audio_path.as_deref(), Some("/tmp/audio.wav"));

        let sentences = db::get_sentences_for_job(&pool, job_id).await.unwrap();
        assert_eq!(sentences.len(), 2);
        assert_eq!(sentences[0].text, "Hello");
        assert_eq!(sentences[1].text, "World");
    }

    #[tokio::test]
    async fn download_failure_sets_error_status() {
        let pool = test_pool().await;
        let job_id = db::create_job(&pool, "https://youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ")
            .await
            .unwrap();

        let mut downloader = MockAudioDownloader::new();
        downloader.expect_download().returning(|_, _| {
            Box::pin(async { Err(DownloadError::Failed("network error".into())) })
        });

        let transcriber = MockTranscriber::new();
        // transcriber should NOT be called

        process_job(
            pool.clone(),
            job_id,
            "https://youtube.com/watch?v=abc".into(),
            "/tmp".into(),
            Arc::new(downloader),
            Arc::new(transcriber),
        )
        .await;

        let job = db::get_job(&pool, job_id).await.unwrap().unwrap();
        assert_eq!(job.status, JobStatus::Error);
        assert_eq!(job.error_message.unwrap(), "Download failed.");

        let sentences = db::get_sentences_for_job(&pool, job_id).await.unwrap();
        assert!(sentences.is_empty());
    }

    #[tokio::test]
    async fn transcription_failure_sets_error_but_keeps_download_info() {
        let pool = test_pool().await;
        let job_id = db::create_job(&pool, "https://youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ")
            .await
            .unwrap();

        let mut downloader = MockAudioDownloader::new();
        downloader.expect_download().returning(|_, _| {
            Box::pin(async {
                Ok(DownloadResult {
                    audio_path: "/tmp/audio.wav".into(),
                    video_path: "/tmp/video.mp4".into(),
                    video_title: "Test Video".into(),
                })
            })
        });

        let mut transcriber = MockTranscriber::new();
        transcriber.expect_transcribe().returning(|_, _| {
            Box::pin(async { Err(TranscribeError::Failed("model load failed".into())) })
        });

        process_job(
            pool.clone(),
            job_id,
            "https://youtube.com/watch?v=abc".into(),
            "/tmp".into(),
            Arc::new(downloader),
            Arc::new(transcriber),
        )
        .await;

        let job = db::get_job(&pool, job_id).await.unwrap().unwrap();
        assert_eq!(job.status, JobStatus::Error);
        assert_eq!(job.audio_path.as_deref(), Some("/tmp/audio.wav"));
        assert_eq!(job.error_message.unwrap(), "Transcription failed.");

        let sentences = db::get_sentences_for_job(&pool, job_id).await.unwrap();
        assert!(sentences.is_empty());
    }
}
