use std::future::Future;
use std::pin::Pin;

#[cfg_attr(test, mockall::automock)]
pub trait MediaExtractor: Send + Sync {
    fn extract_screenshot(
        &self,
        video_path: &str,
        timestamp_secs: f64,
        output_path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), MediaError>> + Send>>;

    fn extract_audio_clip(
        &self,
        audio_path: &str,
        start_secs: f64,
        end_secs: f64,
        output_path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), MediaError>> + Send>>;
}

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("media extraction failed: {0}")]
    Failed(String),
}

/// Format seconds as `HH:MM:SS.mmm` for ffmpeg's `-ss` flag.
pub fn format_ffmpeg_timestamp(secs: f64) -> String {
    let total_secs = secs.floor() as u64;
    let millis = ((secs - secs.floor()) * 1000.0).round() as u64;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

/// Generate deterministic media filenames for a given job + sentence.
/// Returns `(screenshot_filename, audio_clip_filename)`.
pub fn media_filenames(job_id: i64, sentence_id: i64) -> (String, String) {
    (
        format!("yt-mine_{job_id}_{sentence_id}.jpg"),
        format!("yt-mine_{job_id}_{sentence_id}.mp3"),
    )
}

pub struct FfmpegMediaExtractor;

impl MediaExtractor for FfmpegMediaExtractor {
    fn extract_screenshot(
        &self,
        video_path: &str,
        timestamp_secs: f64,
        output_path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), MediaError>> + Send>> {
        let video_path = video_path.to_owned();
        let output_path = output_path.to_owned();
        Box::pin(async move {
            let timestamp = format_ffmpeg_timestamp(timestamp_secs);

            let child = tokio::process::Command::new("ffmpeg")
                .args([
                    "-ss",
                    &timestamp,
                    "-i",
                    &video_path,
                    "-vframes",
                    "1",
                    "-q:v",
                    "2",
                    &output_path,
                    "-y",
                ])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| MediaError::Failed(format!("failed to run ffmpeg: {e}")))?;

            let output = child
                .wait_with_output()
                .await
                .map_err(|e| MediaError::Failed(format!("ffmpeg failed: {e}")))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(MediaError::Failed(format!(
                    "ffmpeg screenshot failed: {stderr}"
                )));
            }

            Ok(())
        })
    }

    fn extract_audio_clip(
        &self,
        audio_path: &str,
        start_secs: f64,
        end_secs: f64,
        output_path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), MediaError>> + Send>> {
        let audio_path = audio_path.to_owned();
        let output_path = output_path.to_owned();
        Box::pin(async move {
            let start = format_ffmpeg_timestamp(start_secs);
            let duration = end_secs - start_secs;
            let duration_str = format!("{duration:.3}");

            let child = tokio::process::Command::new("ffmpeg")
                .args([
                    "-ss",
                    &start,
                    "-i",
                    &audio_path,
                    "-t",
                    &duration_str,
                    "-vn",
                    "-acodec",
                    "libmp3lame",
                    "-q:a",
                    "4",
                    &output_path,
                    "-y",
                ])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| MediaError::Failed(format!("failed to run ffmpeg: {e}")))?;

            let output = child
                .wait_with_output()
                .await
                .map_err(|e| MediaError::Failed(format!("ffmpeg failed: {e}")))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(MediaError::Failed(format!(
                    "ffmpeg audio clip failed: {stderr}"
                )));
            }

            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_ffmpeg_timestamp_zero() {
        assert_eq!(format_ffmpeg_timestamp(0.0), "00:00:00.000");
    }

    #[test]
    fn format_ffmpeg_timestamp_fractional() {
        assert_eq!(format_ffmpeg_timestamp(65.5), "00:01:05.500");
    }

    #[test]
    fn format_ffmpeg_timestamp_hours() {
        assert_eq!(format_ffmpeg_timestamp(3661.25), "01:01:01.250");
    }

    #[test]
    fn media_filenames_format() {
        let (screenshot, audio) = media_filenames(42, 7);
        assert_eq!(screenshot, "yt-mine_42_7.jpg");
        assert_eq!(audio, "yt-mine_42_7.mp3");
    }
}
