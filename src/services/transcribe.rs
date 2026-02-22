use std::future::Future;
use std::pin::Pin;

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
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(
        &self,
        audio_path: String,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<TranscriptSegment>, TranscribeError>> + Send>> {
        let script_path = self.script_path.clone();
        Box::pin(async move {
            let output = tokio::process::Command::new("python3")
                .args([&script_path, &audio_path])
                .output()
                .await
                .map_err(|e| {
                    TranscribeError::Failed(format!("failed to run transcribe script: {e}"))
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(TranscribeError::Failed(format!(
                    "transcribe script failed: {stderr}"
                )));
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
        };
        // Would need a real audio file here
        let result = transcriber
            .transcribe("/tmp/test_audio.wav".into())
            .await;
        // Just check it doesn't panic — actual result depends on environment
        println!("{result:?}");
    }
}
