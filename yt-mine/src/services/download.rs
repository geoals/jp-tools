use std::future::Future;
use std::pin::Pin;

#[cfg_attr(test, mockall::automock)]
pub trait AudioDownloader: Send + Sync {
    fn download(
        &self,
        url: String,
        output_dir: String,
    ) -> Pin<Box<dyn Future<Output = Result<DownloadResult, DownloadError>> + Send>>;
}

#[derive(Debug, Clone)]
pub struct DownloadResult {
    pub audio_path: String,
    pub video_path: String,
    pub video_title: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("download failed: {0}")]
    Failed(String),

    #[error("invalid YouTube URL")]
    InvalidUrl,
}

/// Validates that the URL is a recognized YouTube URL.
pub fn is_valid_youtube_url(url: &str) -> bool {
    let url = url.trim();
    let patterns = [
        "https://www.youtube.com/watch?",
        "http://www.youtube.com/watch?",
        "https://youtube.com/watch?",
        "http://youtube.com/watch?",
        "https://m.youtube.com/watch?",
        "http://m.youtube.com/watch?",
        "https://youtu.be/",
        "http://youtu.be/",
    ];
    patterns.iter().any(|p| url.starts_with(p))
}

/// Extracts the YouTube video ID (11-char slug) from a URL.
///
/// Supports `youtube.com/watch?v=X`, `youtu.be/X`, and mobile/http variants.
/// Returns `None` for non-YouTube URLs or URLs without a valid ID.
pub fn extract_video_id(url: &str) -> Option<String> {
    let url = url.trim();

    // youtu.be/VIDEO_ID or youtu.be/VIDEO_ID?...
    if let Some(rest) = url
        .strip_prefix("https://youtu.be/")
        .or_else(|| url.strip_prefix("http://youtu.be/"))
    {
        let id = rest.split(['?', '&', '/']).next()?;
        return validate_video_id(id);
    }

    // youtube.com/watch?v=VIDEO_ID (with optional www./m. prefix, http/https)
    if !is_valid_youtube_url(url) {
        return None;
    }
    let query = url.split('?').nth(1)?;
    for param in query.split('&') {
        if let Some(value) = param.strip_prefix("v=") {
            return validate_video_id(value);
        }
    }
    None
}

fn validate_video_id(id: &str) -> Option<String> {
    if id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Some(id.to_string())
    } else {
        None
    }
}

pub struct YtDlpDownloader;

impl AudioDownloader for YtDlpDownloader {
    fn download(
        &self,
        url: String,
        output_dir: String,
    ) -> Pin<Box<dyn Future<Output = Result<DownloadResult, DownloadError>> + Send>> {
        Box::pin(async move {
            if !is_valid_youtube_url(&url) {
                return Err(DownloadError::InvalidUrl);
            }

            let output_template = format!("{output_dir}/%(id)s.%(ext)s");

            // Download video at low resolution — 480p is enough for Anki screenshots,
            // and keeps the file small. yt-dlp's -S prefers formats closest to 480p.
            let child = tokio::process::Command::new("yt-dlp")
                .args([
                    "-S",
                    "res:480",
                    "--print",
                    "after_move:filepath",
                    "--print",
                    "title",
                    "-o",
                    &output_template,
                    &url,
                ])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| DownloadError::Failed(format!("failed to run yt-dlp: {e}")))?;

            let output = child
                .wait_with_output()
                .await
                .map_err(|e| DownloadError::Failed(format!("yt-dlp failed: {e}")))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(DownloadError::Failed(format!(
                    "yt-dlp exited with non-zero status: {stderr}"
                )));
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut lines = stdout.lines();

            // yt-dlp prints in execution order: title (info extraction phase)
            // comes before after_move:filepath (post-processing phase)
            let video_title = lines
                .next()
                .ok_or_else(|| DownloadError::Failed("no title in yt-dlp output".into()))?
                .to_string();

            let video_path = lines
                .next()
                .ok_or_else(|| DownloadError::Failed("no filepath in yt-dlp output".into()))?
                .to_string();

            // Extract audio from video via ffmpeg (16kHz mono WAV for whisper)
            let audio_path = {
                let p = std::path::Path::new(&video_path);
                p.with_extension("wav").to_string_lossy().into_owned()
            };

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
                .spawn()
                .map_err(|e| DownloadError::Failed(format!("failed to run ffmpeg: {e}")))?;

            let ffmpeg_output = ffmpeg
                .wait_with_output()
                .await
                .map_err(|e| DownloadError::Failed(format!("ffmpeg failed: {e}")))?;

            if !ffmpeg_output.status.success() {
                let stderr = String::from_utf8_lossy(&ffmpeg_output.stderr);
                return Err(DownloadError::Failed(format!(
                    "ffmpeg audio extraction failed: {stderr}"
                )));
            }

            Ok(DownloadResult {
                audio_path,
                video_path,
                video_title,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_youtube_urls() {
        let valid = [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "http://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com/watch?v=dQw4w9WgXcQ",
            "https://m.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ",
            "http://youtu.be/dQw4w9WgXcQ",
            "https://www.youtube.com/watch?v=abc&list=xyz",
        ];
        for url in valid {
            assert!(is_valid_youtube_url(url), "should be valid: {url}");
        }
    }

    #[test]
    fn invalid_youtube_urls() {
        let invalid = [
            "https://example.com",
            "https://vimeo.com/12345",
            "not a url",
            "",
            "youtube.com/watch?v=abc",
            "https://www.youtube.com/playlist?list=abc",
        ];
        for url in invalid {
            assert!(!is_valid_youtube_url(url), "should be invalid: {url}");
        }
    }

    #[test]
    fn extract_video_id_from_standard_url() {
        assert_eq!(
            extract_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".into()),
        );
    }

    #[test]
    fn extract_video_id_from_short_url() {
        assert_eq!(
            extract_video_id("https://youtu.be/dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".into()),
        );
    }

    #[test]
    fn extract_video_id_from_mobile_url() {
        assert_eq!(
            extract_video_id("https://m.youtube.com/watch?v=dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".into()),
        );
    }

    #[test]
    fn extract_video_id_with_extra_params() {
        assert_eq!(
            extract_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PLrAXtmErZgOeiKm4sgNOknGvNjby9efdf"),
            Some("dQw4w9WgXcQ".into()),
        );
    }

    #[test]
    fn extract_video_id_from_short_url_with_query() {
        assert_eq!(
            extract_video_id("https://youtu.be/dQw4w9WgXcQ?t=42"),
            Some("dQw4w9WgXcQ".into()),
        );
    }

    #[test]
    fn extract_video_id_returns_none_for_invalid_urls() {
        let invalid = [
            "https://example.com",
            "https://vimeo.com/12345",
            "not a url",
            "",
            "https://www.youtube.com/playlist?list=abc",
        ];
        for url in invalid {
            assert_eq!(extract_video_id(url), None, "should be None: {url}");
        }
    }

    #[tokio::test]
    #[ignore = "requires yt-dlp installed"]
    async fn yt_dlp_downloads_audio() {
        let dir = tempfile::tempdir().unwrap();
        let downloader = YtDlpDownloader;
        let result = downloader
            .download(
                // Short public domain video
                "https://www.youtube.com/watch?v=jNQXAC9IVRw".into(),
                dir.path().to_str().unwrap().into(),
            )
            .await;
        let result = result.unwrap();
        assert!(!result.video_title.is_empty());
        assert!(std::path::Path::new(&result.audio_path).exists());
    }
}
