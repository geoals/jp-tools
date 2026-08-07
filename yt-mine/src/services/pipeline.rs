use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::task::JoinHandle;
use tracing::{error, info};

use jp_core::text::sentences::split_sentences;

use crate::db;
use crate::models::{JobStatus, TranscriptSegment};
use crate::services::download::MediaDownloader;
use crate::services::transcribe::{ProgressCallback, Transcriber};

/// Runs the full pipeline for a job: download -> transcribe -> store sentences.
/// Updates job status at each step. On failure, sets status to `error` with a message.
pub async fn process_job(
    pool: SqlitePool,
    job_id: i64,
    youtube_url: String,
    audio_dir: String,
    downloader: Arc<dyn MediaDownloader>,
    transcriber: Arc<dyn Transcriber>,
) {
    // Step 1: Download the audio
    info!(job_id, url = youtube_url, "starting download");
    db::update_job_status(&pool, job_id, &JobStatus::Downloading, None)
        .await
        .ok();

    let audio = match downloader
        .download_audio(youtube_url.clone(), audio_dir.clone())
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

    db::update_job_audio(
        &pool,
        job_id,
        &audio.audio_path,
        &audio.video_title,
        audio.video_duration,
    )
    .await
    .ok();

    // The video is only wanted for the screenshot on a mined card, so it comes
    // down while whisper is already working rather than ahead of it.
    let video_download = {
        let downloader = downloader.clone();
        tokio::spawn(async move { downloader.download_video(youtube_url, audio_dir).await })
    };

    // Step 2: Transcribe — sentences are inserted progressively via the callback,
    // so they appear in the UI during transcription (the frontend polls for updates).
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
            for line in into_sentences(segment) {
                db::insert_sentence(&pool, job_id, &line).await.ok();
            }
        });
        cb_handles.lock().unwrap().push(handle);
    }));

    let segments = match transcriber.transcribe(audio.audio_path, on_progress).await {
        Ok(segments) => segments,
        Err(e) => {
            error!(job_id, error = %e, "transcription failed");
            db::update_job_status(
                &pool,
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

    match video_download.await {
        Ok(Ok(video_path)) => {
            db::update_job_video(&pool, job_id, &video_path).await.ok();
        }
        Ok(Err(e)) => {
            error!(job_id, error = %e, "video download failed");
            db::update_job_status(
                &pool,
                job_id,
                &JobStatus::Error,
                Some("Video download failed."),
            )
            .await
            .ok();
            return;
        }
        Err(e) => {
            error!(job_id, error = %e, "video download task failed");
            db::update_job_status(
                &pool,
                job_id,
                &JobStatus::Error,
                Some("Video download failed."),
            )
            .await
            .ok();
            return;
        }
    }

    info!(job_id, count = segments.len(), "job complete");
    db::update_job_status(&pool, job_id, &JobStatus::Done, None)
        .await
        .ok();
}

/// Drops a subtitle speaker label from the front of a segment.
///
/// Whisper learnt Japanese partly from subtitles written `名前 セリフ`, and on a
/// short window it writes the label too — one podcast gave 234 lines opening
/// `ヤンヤン `, plus 48 `樋口 ` and 3 `深井 `, none of them spoken. Conditioning on
/// the previous text then carries the label forward for as long as it survives,
/// so one hallucination becomes a run of hundreds.
///
/// The ASCII space is what makes this safe to strip: whisper's Japanese output
/// has none of its own, so a leading run of Japanese followed by one is a label
/// and not speech.
fn strip_speaker_label(text: &str) -> &str {
    let Some((label, rest)) = text.split_once(' ') else {
        return text;
    };
    let is_label = !label.is_empty()
        && label.chars().count() <= 8
        && label.chars().all(is_japanese)
        && !rest.trim_start().is_empty();

    if is_label { rest.trim_start() } else { text }
}

fn is_japanese(c: char) -> bool {
    matches!(c,
        '\u{3040}'..='\u{309F}'   // hiragana
        | '\u{30A0}'..='\u{30FF}' // katakana, ー
        | '\u{4E00}'..='\u{9FFF}' // kanji
        | '\u{3005}'              // 々
    )
}

/// One whisper segment, cut into the sentences it holds.
///
/// Whisper is primed with punctuated Japanese so a mined line keeps its 。 and
/// 、 — see whisper-service — and priming also makes it run sentences
/// together: half-minute segments holding eight sentences each, which is not a
/// card. The punctuation is whisper's own and reliable, so the segment is cut
/// on it here rather than mined as one line.
///
/// Times are shared out by character count, since a segment carries no timing
/// inside itself. A line's audio is therefore accurate to a fraction of a
/// second rather than exact — speech rate is near enough uniform within one
/// segment for that to hold.
fn into_sentences(mut segment: TranscriptSegment) -> Vec<TranscriptSegment> {
    segment.text = strip_speaker_label(&segment.text).to_string();

    let sentences = split_sentences(&segment.text);
    if sentences.len() < 2 {
        return vec![segment];
    }

    let total: usize = sentences.iter().map(|s| s.chars().count()).sum();
    if total == 0 {
        return vec![segment];
    }

    let span = segment.end - segment.start;
    let mut out = Vec::with_capacity(sentences.len());
    let mut seen = 0usize;
    for text in sentences {
        let len = text.chars().count();
        let start = segment.start + span * (seen as f64 / total as f64);
        seen += len;
        let end = segment.start + span * (seen as f64 / total as f64);
        out.push(TranscriptSegment { start, end, text });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TranscriptSegment;
    use crate::services::download::{AudioDownload, DownloadError, MockMediaDownloader};
    use crate::services::transcribe::{MockTranscriber, TranscribeError};

    #[test]
    fn a_segment_holding_several_sentences_becomes_several_lines() {
        // What the opening of a video actually looks like: the initial prompt
        // conditions the first window, so whisper runs sentences together
        // there and settles down afterwards.
        let out = into_sentences(TranscriptSegment {
            start: 0.0,
            end: 30.0,
            text:
                "執着する今のあなたは、自分の人生を生きていません。あの人が忘れられなくてつらい。"
                    .into(),
        });

        assert_eq!(out.len(), 2);
        assert_eq!(
            out[0].text,
            "執着する今のあなたは、自分の人生を生きていません。"
        );
        assert_eq!(out[1].text, "あの人が忘れられなくてつらい。");
        // Shared out by length, and the two together still cover the segment.
        assert_eq!(out[0].start, 0.0);
        assert_eq!(out[1].end, 30.0);
        assert!((out[0].end - out[1].start).abs() < 1e-9);
        assert!(out[0].end > out[1].end - out[1].start);
    }

    #[test]
    fn a_segment_that_is_one_sentence_is_left_exactly_as_it_was() {
        let one = TranscriptSegment {
            start: 4.0,
            end: 7.5,
            text: "もう本当に久しぶり。".into(),
        };
        let out = into_sentences(one.clone());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].start, 4.0);
        assert_eq!(out[0].end, 7.5);
        assert_eq!(out[0].text, "もう本当に久しぶり。");
    }

    #[test]
    fn a_hallucinated_speaker_label_is_dropped() {
        let out = into_sentences(TranscriptSegment {
            start: 0.0,
            end: 3.0,
            text: "ヤンヤン 響きが沖縄だもんな。".into(),
        });
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "響きが沖縄だもんな。");
    }

    #[test]
    fn the_label_goes_before_the_segment_is_cut_into_sentences() {
        let out = into_sentences(TranscriptSegment {
            start: 0.0,
            end: 6.0,
            text: "樋口 確かに。そうですね。".into(),
        });
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "確かに。");
        assert_eq!(out[1].text, "そうですね。");
    }

    #[test]
    fn speech_is_left_alone() {
        // No space at all: whisper's ordinary Japanese output.
        assert_eq!(
            strip_speaker_label("何のためかっていうと、"),
            "何のためかっていうと、"
        );
        // A space, but the run before it is not a label.
        assert_eq!(strip_speaker_label("OK でしょ"), "OK でしょ");
        assert_eq!(
            strip_speaker_label("ガソリンスタンドの溝は防災上の理由 です"),
            "ガソリンスタンドの溝は防災上の理由 です"
        );
        // A label with nothing after it is all there is — keep it.
        assert_eq!(strip_speaker_label("ヤンヤン "), "ヤンヤン ");
    }

    async fn test_pool() -> SqlitePool {
        db::create_pool("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn happy_path_downloads_transcribes_and_stores() {
        let pool = test_pool().await;
        let job_id = db::create_job(
            &pool,
            "https://youtube.com/watch?v=dQw4w9WgXcQ",
            "dQw4w9WgXcQ",
        )
        .await
        .unwrap();

        let mut downloader = MockMediaDownloader::new();
        downloader.expect_download_audio().returning(|_, _| {
            Box::pin(async {
                Ok(AudioDownload {
                    audio_path: "/tmp/audio.wav".into(),
                    video_title: "Test Video".into(),
                    video_duration: Some(60.0),
                })
            })
        });
        downloader
            .expect_download_video()
            .returning(|_, _| Box::pin(async { Ok("/tmp/video.mp4".to_string()) }));

        let mut transcriber = MockTranscriber::new();
        transcriber.expect_transcribe().returning(|_, on_progress| {
            Box::pin(async move {
                let segments = vec![
                    TranscriptSegment {
                        start: 0.0,
                        end: 3.0,
                        text: "Hello".into(),
                    },
                    TranscriptSegment {
                        start: 3.0,
                        end: 6.0,
                        text: "World".into(),
                    },
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
        let job_id = db::create_job(
            &pool,
            "https://youtube.com/watch?v=dQw4w9WgXcQ",
            "dQw4w9WgXcQ",
        )
        .await
        .unwrap();

        let mut downloader = MockMediaDownloader::new();
        downloader.expect_download_audio().returning(|_, _| {
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
        let job_id = db::create_job(
            &pool,
            "https://youtube.com/watch?v=dQw4w9WgXcQ",
            "dQw4w9WgXcQ",
        )
        .await
        .unwrap();

        let mut downloader = MockMediaDownloader::new();
        downloader.expect_download_audio().returning(|_, _| {
            Box::pin(async {
                Ok(AudioDownload {
                    audio_path: "/tmp/audio.wav".into(),
                    video_title: "Test Video".into(),
                    video_duration: Some(60.0),
                })
            })
        });
        downloader
            .expect_download_video()
            .returning(|_, _| Box::pin(async { Ok("/tmp/video.mp4".to_string()) }));

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
