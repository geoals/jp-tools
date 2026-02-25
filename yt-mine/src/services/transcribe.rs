use std::future::Future;
use std::pin::Pin;

use crate::models::TranscriptSegment;

/// Called for each segment as it arrives during transcription.
/// Receives the segment and the cumulative count so far.
pub type ProgressCallback = Box<dyn Fn(TranscriptSegment, usize) + Send + Sync>;

#[cfg_attr(test, mockall::automock)]
pub trait Transcriber: Send + Sync {
    fn transcribe(
        &self,
        audio_path: String,
        on_progress: Option<ProgressCallback>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<TranscriptSegment>, TranscribeError>> + Send>>;
}

#[derive(Debug, thiserror::Error)]
pub enum TranscribeError {
    #[error("transcription failed: {0}")]
    Failed(String),
}

/// Transcribes audio by uploading it to a remote whisper-service instance.
/// The service streams NDJSON segments back as they complete.
pub struct RemoteTranscriber {
    base_url: String,
    client: reqwest::Client,
}

impl std::fmt::Debug for RemoteTranscriber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteTranscriber")
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl RemoteTranscriber {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }
}

impl Transcriber for RemoteTranscriber {
    fn transcribe(
        &self,
        audio_path: String,
        on_progress: Option<ProgressCallback>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<TranscriptSegment>, TranscribeError>> + Send>> {
        let url = format!("{}/transcribe", self.base_url);
        let client = self.client.clone();

        Box::pin(async move {
            let file_bytes = tokio::fs::read(&audio_path).await.map_err(|e| {
                TranscribeError::Failed(format!("failed to read audio file {audio_path}: {e}"))
            })?;

            let file_name = std::path::Path::new(&audio_path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();

            let part = reqwest::multipart::Part::bytes(file_bytes).file_name(file_name);
            let form = reqwest::multipart::Form::new().part("audio", part);

            let response = client
                .post(&url)
                .multipart(form)
                .send()
                .await
                .map_err(|e| {
                    TranscribeError::Failed(format!("failed to send request to whisper service: {e}"))
                })?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(TranscribeError::Failed(format!(
                    "whisper service returned {status}: {body}"
                )));
            }

            // Stream NDJSON response via chunk(): one TranscriptSegment per line.
            // No extra deps needed — reqwest's chunk() returns Option<Bytes>.
            let mut segments = Vec::new();
            let mut buffer = String::new();
            let mut response = response;

            while let Some(chunk) = response.chunk().await.map_err(|e| {
                TranscribeError::Failed(format!("error reading response stream: {e}"))
            })? {
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Process complete lines
                while let Some(newline_pos) = buffer.find('\n') {
                    let line = buffer[..newline_pos].trim().to_string();
                    buffer = buffer[newline_pos + 1..].to_string();

                    if line.is_empty() {
                        continue;
                    }

                    let segment: TranscriptSegment =
                        serde_json::from_str(&line).map_err(|e| {
                            TranscribeError::Failed(format!(
                                "failed to parse NDJSON segment: {e}"
                            ))
                        })?;

                    segments.push(segment.clone());
                    if let Some(cb) = &on_progress {
                        cb(segment, segments.len());
                    }
                }
            }

            // Handle any remaining data in buffer (last line may lack trailing newline)
            let remaining = buffer.trim();
            if !remaining.is_empty() {
                let segment: TranscriptSegment =
                    serde_json::from_str(remaining).map_err(|e| {
                        TranscribeError::Failed(format!(
                            "failed to parse final NDJSON segment: {e}"
                        ))
                    })?;
                segments.push(segment.clone());
                if let Some(cb) = &on_progress {
                    cb(segment, segments.len());
                }
            }

            Ok(segments)
        })
    }
}
