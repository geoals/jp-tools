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

            let output = tokio::process::Command::new("yt-dlp")
                .args([
                    "-x",
                    "--audio-format",
                    "wav",
                    "--print",
                    "after_move:filepath",
                    "--print",
                    "title",
                    "-o",
                    &output_template,
                    &url,
                ])
                .output()
                .await
                .map_err(|e| DownloadError::Failed(format!("failed to run yt-dlp: {e}")))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(DownloadError::Failed(format!("yt-dlp failed: {stderr}")));
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut lines = stdout.lines();

            let audio_path = lines
                .next()
                .ok_or_else(|| DownloadError::Failed("no filepath in yt-dlp output".into()))?
                .to_string();

            let video_title = lines
                .next()
                .ok_or_else(|| DownloadError::Failed("no title in yt-dlp output".into()))?
                .to_string();

            Ok(DownloadResult {
                audio_path,
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
