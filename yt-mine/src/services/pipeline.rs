//! What happens to a video after it is submitted.
//!
//! Two passes, and the split between them is the whole point. `process_job`
//! fetches YouTube's own captions — a second or two for the entire video, so
//! the line list is readable immediately no matter how far in the line is.
//! `refine_window` is whisper, over one minute of audio, run only where a card
//! is actually being made. The old shape (download everything, transcribe from
//! 0:00) survives as `transcribe_whole_video`, for a video with no Japanese
//! captions at all.

use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::db::{self, SentenceOrigin};
use crate::models::JobStatus;
use crate::services::captions::{CaptionError, CaptionSource};
use crate::services::clip::ClipFetcher;
use crate::services::download::AudioDownloader;
use crate::services::transcribe::{ProgressCallback, Transcriber};

/// How much of the video around a timestamp whisper is given.
///
/// Wide enough that the line asked for is never at the edge — whisper's
/// segmentation needs the run-up — and short enough to transcribe in seconds.
pub const WINDOW_BEFORE: f64 = 25.0;
pub const WINDOW_AFTER: f64 = 25.0;

pub struct Pipeline {
    pub pool: SqlitePool,
    pub captions: Arc<dyn CaptionSource>,
    pub clips: Arc<dyn ClipFetcher>,
    pub downloader: Arc<dyn AudioDownloader>,
    pub transcriber: Arc<dyn Transcriber>,
    pub audio_dir: String,
}

/// Fetch the captions and store them as the video's lines.
///
/// Falls back to downloading and transcribing the whole video when YouTube has
/// no Japanese track.
pub async fn process_job(p: Arc<Pipeline>, job_id: i64, youtube_url: String) {
    info!(job_id, url = youtube_url, "fetching captions");
    db::update_job_status(&p.pool, job_id, &JobStatus::Fetching, None)
        .await
        .ok();

    let result = p
        .captions
        .fetch(youtube_url.clone(), p.audio_dir.clone())
        .await;

    let captions = match result {
        Ok(c) => c,
        Err(CaptionError::None) => {
            info!(job_id, "no Japanese captions, transcribing the whole video");
            transcribe_whole_video(p, job_id, youtube_url).await;
            return;
        }
        Err(e) => {
            error!(job_id, error = %e, "caption fetch failed");
            db::update_job_status(
                &p.pool,
                job_id,
                &JobStatus::Error,
                Some("Could not read this video's captions."),
            )
            .await
            .ok();
            return;
        }
    };

    // No audio or video path: nothing has been downloaded, and nothing will be
    // until a window is sharpened or a card is exported.
    db::update_job_title(
        &p.pool,
        job_id,
        &captions.video_title,
        captions.video_duration,
    )
    .await
    .ok();

    if let Err(e) = db::insert_sentences(
        &p.pool,
        job_id,
        &captions.segments,
        &SentenceOrigin::captions(),
    )
    .await
    {
        error!(job_id, error = %e, "failed to store captions");
        db::update_job_status(
            &p.pool,
            job_id,
            &JobStatus::Error,
            Some("Could not store this video's captions."),
        )
        .await
        .ok();
        return;
    }

    info!(
        job_id,
        count = captions.segments.len(),
        manual = captions.manual,
        "captions ready"
    );
    db::update_job_status(&p.pool, job_id, &JobStatus::Done, None)
        .await
        .ok();
}

/// Replace the caption lines around `at` with whisper's reading of the audio.
///
/// The clip it downloads stays on those lines, so the card's screenshot and
/// audio come out of the same file rather than a second download.
pub async fn refine_window(p: Arc<Pipeline>, job_id: i64, youtube_url: String, at: f64) {
    let start = (at - WINDOW_BEFORE).max(0.0);
    let end = at + WINDOW_AFTER;

    db::set_refine_state(&p.pool, job_id, Some("running"), Some(at))
        .await
        .ok();

    let pool = p.pool.clone();
    let fail = |message: &str| {
        let pool = pool.clone();
        let message = message.to_string();
        async move {
            db::set_refine_state(&pool, job_id, Some(&message), Some(at))
                .await
                .ok();
        }
    };

    info!(job_id, start, end, "sharpening window");
    let clip = match p
        .clips
        .fetch(youtube_url, start, end, p.audio_dir.clone())
        .await
    {
        Ok(c) => c,
        Err(e) => {
            error!(job_id, error = %e, "clip download failed");
            fail("Could not download that part of the video.").await;
            return;
        }
    };

    let segments = match p
        .transcriber
        .transcribe(clip.audio_path.clone(), None)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            error!(job_id, error = %e, "window transcription failed");
            fail("Transcription failed.").await;
            return;
        }
    };

    // Whisper timestamps are relative to the clip; everything stored is
    // absolute, so the window's start goes back on before they are compared to
    // anything.
    let heard: Vec<_> = segments
        .into_iter()
        .map(|mut s| {
            s.start += clip.start;
            s.end += clip.start;
            s
        })
        .filter(|s| s.end > start && s.start < end)
        .collect();

    let existing = db::get_sentences_for_job(&p.pool, job_id)
        .await
        .unwrap_or_default();
    let absolute = fit_to_lines(heard, &existing, start, end);

    if absolute.is_empty() {
        warn!(
            job_id,
            "window transcribed to nothing, keeping the captions"
        );
        db::set_refine_state(&p.pool, job_id, Some("done"), Some(at))
            .await
            .ok();
        return;
    }

    let origin = SentenceOrigin {
        source: "whisper",
        clip: Some(&clip),
    };
    let replaced = db::delete_caption_sentences_in_window(&p.pool, job_id, start, end).await;
    if let Err(e) = replaced {
        error!(job_id, error = %e, "failed to clear the window");
        fail("Could not update those lines.").await;
        return;
    }
    if let Err(e) = db::insert_sentences(&p.pool, job_id, &absolute, &origin).await {
        error!(job_id, error = %e, "failed to store the window");
        fail("Could not store those lines.").await;
        return;
    }

    info!(job_id, count = absolute.len(), "window sharpened");
    db::set_refine_state(&p.pool, job_id, Some("done"), Some(at))
        .await
        .ok();
}

/// Give whisper's words the captions' sentence boundaries.
///
/// The two transcripts are each better at one half of the job. Whisper hears
/// the words — 断捨離 where the captions had 断捨離れ — but this model returns
/// breath-length fragments with no punctuation, and a card whose sentence is
/// "で" is worthless. YouTube's captions are the opposite: 。 in the right
/// places, wrong kanji. So each whisper fragment joins the caption line its
/// midpoint falls in, and the line keeps that shape.
///
/// With no caption lines to fit to — a video that had none, so the whole thing
/// went through whisper — the fragments are left as they are.
fn fit_to_lines(
    heard: Vec<crate::models::TranscriptSegment>,
    existing: &[crate::models::Sentence],
    start: f64,
    end: f64,
) -> Vec<crate::models::TranscriptSegment> {
    let lines: Vec<&crate::models::Sentence> = existing
        .iter()
        .filter(|s| s.source == "captions" && s.start_time < end && s.end_time > start)
        .collect();
    if lines.is_empty() {
        return heard;
    }

    let mut out: Vec<crate::models::TranscriptSegment> = Vec::new();
    let mut current: Option<usize> = None;
    for fragment in heard {
        let middle = (fragment.start + fragment.end) / 2.0;
        let line = lines
            .iter()
            .rposition(|l| l.start_time <= middle)
            .unwrap_or(0);

        match (current, out.last_mut()) {
            (Some(prev), Some(open)) if prev == line => {
                open.text.push_str(&fragment.text);
                open.end = fragment.end;
            }
            _ => {
                current = Some(line);
                out.push(fragment);
            }
        }
    }
    out
}

/// The old whole-video pass, kept for a video YouTube has no Japanese
/// captions for. Sentences are inserted as they arrive so they appear while
/// it runs.
async fn transcribe_whole_video(p: Arc<Pipeline>, job_id: i64, youtube_url: String) {
    db::update_job_status(&p.pool, job_id, &JobStatus::Downloading, None)
        .await
        .ok();

    let download = match p
        .downloader
        .download(youtube_url, p.audio_dir.clone())
        .await
    {
        Ok(result) => result,
        Err(e) => {
            error!(job_id, error = %e, "download failed");
            db::update_job_status(&p.pool, job_id, &JobStatus::Error, Some("Download failed."))
                .await
                .ok();
            return;
        }
    };

    db::update_job_download(
        &p.pool,
        job_id,
        &download.audio_path,
        &download.video_title,
        &download.video_path,
        download.video_duration,
    )
    .await
    .ok();

    db::update_job_status(&p.pool, job_id, &JobStatus::Transcribing, None)
        .await
        .ok();

    let progress_pool = p.pool.clone();
    let write_handles: Arc<std::sync::Mutex<Vec<JoinHandle<()>>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let cb_handles = write_handles.clone();

    let on_progress: Option<ProgressCallback> = Some(Box::new(move |segment, _count| {
        let pool = progress_pool.clone();
        let handle = tokio::spawn(async move {
            let origin = SentenceOrigin {
                source: "whisper",
                clip: None,
            };
            db::insert_sentence(&pool, job_id, &segment, &origin)
                .await
                .ok();
        });
        cb_handles.lock().unwrap().push(handle);
    }));

    let segments = match p
        .transcriber
        .transcribe(download.audio_path, on_progress)
        .await
    {
        Ok(segments) => segments,
        Err(e) => {
            error!(job_id, error = %e, "transcription failed");
            db::update_job_status(
                &p.pool,
                job_id,
                &JobStatus::Error,
                Some("Transcription failed."),
            )
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
    db::update_job_status(&p.pool, job_id, &JobStatus::Done, None)
        .await
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TranscriptSegment;
    use crate::services::captions::{CaptionResult, MockCaptionSource};
    use crate::services::clip::{Clip, MockClipFetcher};
    use crate::services::download::{DownloadError, DownloadResult, MockAudioDownloader};
    use crate::services::transcribe::{MockTranscriber, TranscribeError};

    async fn test_pool() -> SqlitePool {
        db::create_pool("sqlite::memory:").await.unwrap()
    }

    fn pipeline(
        pool: SqlitePool,
        captions: MockCaptionSource,
        clips: MockClipFetcher,
        downloader: MockAudioDownloader,
        transcriber: MockTranscriber,
    ) -> Arc<Pipeline> {
        Arc::new(Pipeline {
            pool,
            captions: Arc::new(captions),
            clips: Arc::new(clips),
            downloader: Arc::new(downloader),
            transcriber: Arc::new(transcriber),
            audio_dir: "/tmp".into(),
        })
    }

    fn seg(start: f64, end: f64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            start,
            end,
            text: text.into(),
        }
    }

    async fn a_job(pool: &SqlitePool) -> i64 {
        db::create_job(
            pool,
            "https://youtube.com/watch?v=dQw4w9WgXcQ",
            "dQw4w9WgXcQ",
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn captions_become_the_lines_without_downloading_anything() {
        let pool = test_pool().await;
        let job_id = a_job(&pool).await;

        let mut captions = MockCaptionSource::new();
        captions.expect_fetch().returning(|_, _| {
            Box::pin(async {
                Ok(CaptionResult {
                    video_title: "Test Video".into(),
                    video_duration: Some(600.0),
                    segments: vec![
                        seg(0.0, 3.0, "皆さんこんにちは。"),
                        seg(3.0, 6.0, "元気ですか。"),
                    ],
                    manual: false,
                })
            })
        });

        // Neither of these may be called: the whole point is that no audio is
        // fetched until a card is being made.
        let downloader = MockAudioDownloader::new();
        let transcriber = MockTranscriber::new();

        let p = pipeline(
            pool.clone(),
            captions,
            MockClipFetcher::new(),
            downloader,
            transcriber,
        );
        process_job(p, job_id, "https://youtube.com/watch?v=abc".into()).await;

        let job = db::get_job(&pool, job_id).await.unwrap().unwrap();
        assert_eq!(job.status, JobStatus::Done);
        assert_eq!(job.video_title.as_deref(), Some("Test Video"));
        assert_eq!(job.audio_path, None);

        let sentences = db::get_sentences_for_job(&pool, job_id).await.unwrap();
        assert_eq!(sentences.len(), 2);
        assert_eq!(sentences[0].source, "captions");
    }

    #[tokio::test]
    async fn a_video_without_captions_falls_back_to_the_whole_video() {
        let pool = test_pool().await;
        let job_id = a_job(&pool).await;

        let mut captions = MockCaptionSource::new();
        captions
            .expect_fetch()
            .returning(|_, _| Box::pin(async { Err(CaptionError::None) }));

        let mut downloader = MockAudioDownloader::new();
        downloader.expect_download().returning(|_, _| {
            Box::pin(async {
                Ok(DownloadResult {
                    audio_path: "/tmp/audio.wav".into(),
                    video_path: "/tmp/video.mp4".into(),
                    video_title: "Test Video".into(),
                    video_duration: Some(60.0),
                })
            })
        });

        let mut transcriber = MockTranscriber::new();
        transcriber.expect_transcribe().returning(|_, on_progress| {
            Box::pin(async move {
                let segments = vec![seg(0.0, 3.0, "あ。")];
                if let Some(cb) = &on_progress {
                    cb(segments[0].clone(), 1);
                }
                Ok(segments)
            })
        });

        let p = pipeline(
            pool.clone(),
            captions,
            MockClipFetcher::new(),
            downloader,
            transcriber,
        );
        process_job(p, job_id, "https://youtube.com/watch?v=abc".into()).await;

        let job = db::get_job(&pool, job_id).await.unwrap().unwrap();
        assert_eq!(job.status, JobStatus::Done);
        assert_eq!(job.audio_path.as_deref(), Some("/tmp/audio.wav"));

        let sentences = db::get_sentences_for_job(&pool, job_id).await.unwrap();
        assert_eq!(sentences.len(), 1);
        assert_eq!(sentences[0].source, "whisper");
    }

    #[tokio::test]
    async fn download_failure_sets_error_status() {
        let pool = test_pool().await;
        let job_id = a_job(&pool).await;

        let mut captions = MockCaptionSource::new();
        captions
            .expect_fetch()
            .returning(|_, _| Box::pin(async { Err(CaptionError::None) }));

        let mut downloader = MockAudioDownloader::new();
        downloader.expect_download().returning(|_, _| {
            Box::pin(async { Err(DownloadError::Failed("network error".into())) })
        });

        let p = pipeline(
            pool.clone(),
            captions,
            MockClipFetcher::new(),
            downloader,
            MockTranscriber::new(),
        );
        process_job(p, job_id, "https://youtube.com/watch?v=abc".into()).await;

        let job = db::get_job(&pool, job_id).await.unwrap().unwrap();
        assert_eq!(job.status, JobStatus::Error);
        assert_eq!(job.error_message.unwrap(), "Download failed.");
    }

    #[tokio::test]
    async fn transcription_failure_sets_error_but_keeps_download_info() {
        let pool = test_pool().await;
        let job_id = a_job(&pool).await;

        let mut captions = MockCaptionSource::new();
        captions
            .expect_fetch()
            .returning(|_, _| Box::pin(async { Err(CaptionError::None) }));

        let mut downloader = MockAudioDownloader::new();
        downloader.expect_download().returning(|_, _| {
            Box::pin(async {
                Ok(DownloadResult {
                    audio_path: "/tmp/audio.wav".into(),
                    video_path: "/tmp/video.mp4".into(),
                    video_title: "Test Video".into(),
                    video_duration: Some(60.0),
                })
            })
        });

        let mut transcriber = MockTranscriber::new();
        transcriber.expect_transcribe().returning(|_, _| {
            Box::pin(async { Err(TranscribeError::Failed("model load failed".into())) })
        });

        let p = pipeline(
            pool.clone(),
            captions,
            MockClipFetcher::new(),
            downloader,
            transcriber,
        );
        process_job(p, job_id, "https://youtube.com/watch?v=abc".into()).await;

        let job = db::get_job(&pool, job_id).await.unwrap().unwrap();
        assert_eq!(job.status, JobStatus::Error);
        assert_eq!(job.audio_path.as_deref(), Some("/tmp/audio.wav"));
        assert_eq!(job.error_message.unwrap(), "Transcription failed.");
    }

    #[tokio::test]
    async fn refining_replaces_the_captions_in_the_window_and_nothing_else() {
        let pool = test_pool().await;
        let job_id = a_job(&pool).await;

        db::insert_sentences(
            &pool,
            job_id,
            &[
                seg(10.0, 13.0, "外側の行。"),
                seg(100.0, 103.0, "窓の中の行。"),
                seg(300.0, 303.0, "ずっと後の行。"),
            ],
            &SentenceOrigin::captions(),
        )
        .await
        .unwrap();

        let mut clips = MockClipFetcher::new();
        clips.expect_fetch().returning(|_, start, _, _| {
            Box::pin(async move {
                Ok(Clip {
                    video_path: "/tmp/clip.mp4".into(),
                    audio_path: "/tmp/clip.wav".into(),
                    start,
                })
            })
        });

        // Whisper counts from the clip's own zero.
        let mut transcriber = MockTranscriber::new();
        transcriber.expect_transcribe().returning(|_, _| {
            Box::pin(async { Ok(vec![seg(24.0, 27.0, "はっきりした行。")]) })
        });

        let p = pipeline(
            pool.clone(),
            MockCaptionSource::new(),
            clips,
            MockAudioDownloader::new(),
            transcriber,
        );
        refine_window(p, job_id, "https://youtube.com/watch?v=abc".into(), 100.0).await;

        let sentences = db::get_sentences_for_job(&pool, job_id).await.unwrap();
        let texts: Vec<&str> = sentences.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["外側の行。", "はっきりした行。", "ずっと後の行。"]);

        let refined = &sentences[1];
        assert_eq!(refined.source, "whisper");
        // 75.0 (window start) + 24.0 (clip-relative)
        assert!((refined.start_time - 99.0).abs() < 1e-9);
        assert_eq!(refined.clip_audio_path.as_deref(), Some("/tmp/clip.wav"));
        assert_eq!(refined.clip_start, Some(75.0));

        let job = db::get_job(&pool, job_id).await.unwrap().unwrap();
        assert_eq!(job.refine_state.as_deref(), Some("done"));
    }

    #[test]
    fn whispers_fragments_take_the_captions_sentence_boundaries() {
        let sentence = |id: i64, start: f64, end: f64| crate::models::Sentence {
            id,
            job_id: 1,
            text: String::new(),
            start_time: start,
            end_time: end,
            created_at: String::new(),
            source: "captions".into(),
            clip_path: None,
            clip_audio_path: None,
            clip_start: None,
        };

        let fitted = fit_to_lines(
            vec![
                seg(100.0, 102.0, "買われないものが結構あって"),
                seg(102.0, 104.0, "やっぱり高かったやつとか"),
                seg(106.0, 108.0, "次の文です"),
            ],
            &[sentence(1, 99.0, 105.0), sentence(2, 105.0, 110.0)],
            90.0,
            120.0,
        );

        assert_eq!(fitted.len(), 2);
        assert_eq!(
            fitted[0].text,
            "買われないものが結構あってやっぱり高かったやつとか"
        );
        assert_eq!(fitted[0].end, 104.0);
        assert_eq!(fitted[1].text, "次の文です");
    }

    #[test]
    fn with_no_captions_to_fit_to_whispers_own_segmentation_stands() {
        let heard = vec![seg(0.0, 1.0, "あ"), seg(1.0, 2.0, "い")];
        let fitted = fit_to_lines(heard, &[], 0.0, 10.0);
        assert_eq!(fitted.len(), 2);
    }

    #[tokio::test]
    async fn a_failed_refine_leaves_the_captions_alone() {
        let pool = test_pool().await;
        let job_id = a_job(&pool).await;

        db::insert_sentences(
            &pool,
            job_id,
            &[seg(100.0, 103.0, "窓の中の行。")],
            &SentenceOrigin::captions(),
        )
        .await
        .unwrap();

        let mut clips = MockClipFetcher::new();
        clips.expect_fetch().returning(|_, _, _, _| {
            Box::pin(async { Err(crate::services::clip::ClipError::Failed("nope".into())) })
        });

        let p = pipeline(
            pool.clone(),
            MockCaptionSource::new(),
            clips,
            MockAudioDownloader::new(),
            MockTranscriber::new(),
        );
        refine_window(p, job_id, "https://youtube.com/watch?v=abc".into(), 100.0).await;

        let sentences = db::get_sentences_for_job(&pool, job_id).await.unwrap();
        assert_eq!(sentences.len(), 1);
        assert_eq!(sentences[0].text, "窓の中の行。");
        let job = db::get_job(&pool, job_id).await.unwrap().unwrap();
        assert!(job.refine_state.unwrap().starts_with("Could not"));
    }
}
