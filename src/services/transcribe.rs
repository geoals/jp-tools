use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;

use crate::models::TranscriptSegment;

#[cfg_attr(test, mockall::automock)]
pub trait Transcriber: Send + Sync {
    fn transcribe(
        &self,
        audio_path: String,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<TranscriptSegment>, TranscribeError>> + Send>>;
}

#[derive(Debug, thiserror::Error)]
pub enum TranscribeError {
    #[error("transcription failed: {0}")]
    Failed(String),
}

/// Parses the JSON output from the transcribe.py script.
pub fn parse_transcript_json(json: &str) -> Result<Vec<TranscriptSegment>, TranscribeError> {
    serde_json::from_str(json)
        .map_err(|e| TranscribeError::Failed(format!("failed to parse transcript JSON: {e}")))
}

pub struct WhisperTranscriber {
    pub script_path: String,
    pub cpu_threads: u32,
    pub device: String,
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(
        &self,
        audio_path: String,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<TranscriptSegment>, TranscribeError>> + Send>> {
        let script_path = self.script_path.clone();
        let cpu_threads = self.cpu_threads.to_string();
        let device = self.device.clone();
        Box::pin(async move {
            // Inherit stderr so model downloads, progress, and warnings
            // are streamed to the server logs in real time.
            // script_path is executed directly — either transcribe.py (via shebang)
            // or a wrapper like transcribe-docker.sh.
            let child = tokio::process::Command::new(&script_path)
                .args([&audio_path, &cpu_threads, &device])
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|e| {
                    TranscribeError::Failed(format!("failed to run transcribe script: {e}"))
                })?;

            let output = child.wait_with_output().await.map_err(|e| {
                TranscribeError::Failed(format!("transcribe script failed: {e}"))
            })?;

            if !output.status.success() {
                return Err(TranscribeError::Failed(
                    "transcribe script exited with non-zero status (see logs above)".into(),
                ));
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            parse_transcript_json(&stdout)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_transcript_json() {
        let json = r#"[
            {"start": 0.0, "end": 3.2, "text": "今日は皆さんにお知らせがあります"},
            {"start": 3.5, "end": 6.1, "text": "来週から新しいプロジェクトが始まります"}
        ]"#;

        let segments = parse_transcript_json(json).unwrap();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text, "今日は皆さんにお知らせがあります");
        assert_eq!(segments[0].start, 0.0);
        assert_eq!(segments[0].end, 3.2);
        assert_eq!(segments[1].text, "来週から新しいプロジェクトが始まります");
    }

    #[test]
    fn parse_empty_transcript() {
        let segments = parse_transcript_json("[]").unwrap();
        assert!(segments.is_empty());
    }

    #[test]
    fn parse_invalid_json_returns_error() {
        let result = parse_transcript_json("not json");
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires Python + faster-whisper installed"]
    async fn whisper_transcriber_integration() {
        let transcriber = WhisperTranscriber {
            script_path: "scripts/transcribe.py".into(),
            cpu_threads: 0,
            device: "auto".into(),
        };
        let result = transcriber
            .transcribe("/tmp/test_audio.wav".into())
            .await;
        println!("{result:?}");
    }
}
