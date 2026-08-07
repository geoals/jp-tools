//! A window of a video, downloaded on its own.
//!
//! Nothing here ever needs the whole file: whisper sharpens a minute around
//! the line being read, and a card wants a screenshot and a few seconds of
//! audio. `yt-dlp --download-sections` fetches exactly that, so a line at
//! 31:00 costs the same as a line at 0:30.

use std::future::Future;
use std::pin::Pin;

#[derive(Debug, Clone)]
pub struct Clip {
    pub video_path: String,
    pub audio_path: String,
    /// Where the clip starts in the video. Every timestamp stored is absolute,
    /// so this is what has to come off one before ffmpeg is pointed at the
    /// clip, and what has to be added back to whisper's output.
    pub start: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum ClipError {
    #[error("clip download failed: {0}")]
    Failed(String),
}

#[cfg_attr(test, mockall::automock)]
pub trait ClipFetcher: Send + Sync {
    fn fetch(
        &self,
        url: String,
        start: f64,
        end: f64,
        output_dir: String,
    ) -> Pin<Box<dyn Future<Output = Result<Clip, ClipError>> + Send>>;
}

pub struct YtDlpClipFetcher;

impl ClipFetcher for YtDlpClipFetcher {
    fn fetch(
        &self,
        url: String,
        start: f64,
        end: f64,
        output_dir: String,
    ) -> Pin<Box<dyn Future<Output = Result<Clip, ClipError>> + Send>> {
        Box::pin(async move {
            let id = crate::services::download::extract_video_id(&url)
                .ok_or(ClipError::Failed("not a YouTube URL".into()))?;
            let start = start.max(0.0);
            let stem = format!("{output_dir}/{id}_{:.0}_{:.0}", start, end);
            let video_path = format!("{stem}.mp4");
            let audio_path = format!("{stem}.wav");

            if tokio::fs::try_exists(&audio_path).await.unwrap_or(false) {
                return Ok(Clip {
                    video_path,
                    audio_path,
                    start,
                });
            }

            let section = format!("*{start:.2}-{end:.2}");
            let output = tokio::process::Command::new("yt-dlp")
                .args([
                    "--no-update",
                    "-S",
                    "res:480",
                    "--merge-output-format",
                    "mp4",
                    "--download-sections",
                    &section,
                    // Without this the cut lands on the previous keyframe, so
                    // the clip silently starts earlier than asked and every
                    // timestamp derived from it is off by an unknown amount.
                    "--force-keyframes-at-cuts",
                    "-o",
                    &video_path,
                    &url,
                ])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .await
                .map_err(|e| ClipError::Failed(format!("failed to run yt-dlp: {e}")))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(ClipError::Failed(format!("yt-dlp failed: {stderr}")));
            }

            let ffmpeg = tokio::process::Command::new("ffmpeg")
                .args([
                    "-i",
                    &video_path,
                    "-vn",
                    "-acodec",
                    "pcm_s16le",
                    "-ar",
                    "16000",
                    "-ac",
                    "1",
                    &audio_path,
                    "-y",
                ])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .await
                .map_err(|e| ClipError::Failed(format!("failed to run ffmpeg: {e}")))?;

            if !ffmpeg.status.success() {
                let stderr = String::from_utf8_lossy(&ffmpeg.stderr);
                return Err(ClipError::Failed(format!(
                    "ffmpeg audio extraction failed: {stderr}"
                )));
            }

            Ok(Clip {
                video_path,
                audio_path,
                start,
            })
        })
    }
}
