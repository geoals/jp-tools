use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::info;

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
/// Handles both segment arrays and error objects from worker mode.
pub fn parse_transcript_json(json: &str) -> Result<Vec<TranscriptSegment>, TranscribeError> {
    // Check for error response from worker mode: {"error": "..."}
    if let Ok(err_obj) = serde_json::from_str::<serde_json::Value>(json) {
        if let Some(err_msg) = err_obj.get("error").and_then(|v| v.as_str()) {
            return Err(TranscribeError::Failed(err_msg.to_string()));
        }
    }

    serde_json::from_str(json)
        .map_err(|e| TranscribeError::Failed(format!("failed to parse transcript JSON: {e}")))
}

struct WorkerProcess {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

/// Persistent whisper worker — spawns the transcription script once at startup
/// in `--worker` mode, keeping the model loaded in RAM. Subsequent transcriptions
/// send audio paths over stdin and read JSON results from stdout.
pub struct WhisperWorker {
    process: Arc<Mutex<WorkerProcess>>,
}

impl std::fmt::Debug for WhisperWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WhisperWorker").finish_non_exhaustive()
    }
}

impl WhisperWorker {
    /// Spawns the whisper worker subprocess and waits for it to signal READY
    /// (indicating the model is loaded and ready for transcription requests).
    pub async fn spawn(script_path: &str) -> Result<Self, TranscribeError> {
        info!(script_path, "spawning whisper worker");

        let mut child = tokio::process::Command::new(script_path)
            .arg("--worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                TranscribeError::Failed(format!("failed to spawn whisper worker: {e}"))
            })?;

        let stdin = child.stdin.take().expect("stdin was piped");
        let stdout = child.stdout.take().expect("stdout was piped");
        let mut stdout = BufReader::new(stdout);

        // Wait for the READY signal from the worker
        let mut ready_line = String::new();
        stdout.read_line(&mut ready_line).await.map_err(|e| {
            TranscribeError::Failed(format!("failed to read READY from whisper worker: {e}"))
        })?;

        if ready_line.trim() != "READY" {
            return Err(TranscribeError::Failed(format!(
                "expected READY from whisper worker, got: {ready_line:?}"
            )));
        }

        info!("whisper worker ready");

        Ok(Self {
            process: Arc::new(Mutex::new(WorkerProcess { stdin, stdout })),
        })
    }
}

impl Transcriber for WhisperWorker {
    fn transcribe(
        &self,
        audio_path: String,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<TranscriptSegment>, TranscribeError>> + Send>> {
        let process = Arc::clone(&self.process);

        Box::pin(async move {
            let mut worker = process.lock().await;

            // Send the audio path
            worker
                .stdin
                .write_all(format!("{audio_path}\n").as_bytes())
                .await
                .map_err(|e| {
                    TranscribeError::Failed(format!("failed to write to whisper worker: {e}"))
                })?;
            worker.stdin.flush().await.map_err(|e| {
                TranscribeError::Failed(format!("failed to flush whisper worker stdin: {e}"))
            })?;

            // Read one line of JSON response
            let mut response = String::new();
            worker
                .stdout
                .read_line(&mut response)
                .await
                .map_err(|e| {
                    TranscribeError::Failed(format!(
                        "failed to read response from whisper worker: {e}"
                    ))
                })?;

            if response.is_empty() {
                return Err(TranscribeError::Failed(
                    "whisper worker closed stdout unexpectedly".into(),
                ));
            }

            parse_transcript_json(response.trim())
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

    #[test]
    fn parse_worker_error_response() {
        let json = r#"{"error": "file not found: /tmp/missing.wav"}"#;
        let result = parse_transcript_json(json);
        let err = result.unwrap_err();
        assert!(err.to_string().contains("file not found"));
    }

    #[tokio::test]
    async fn worker_spawn_and_transcribe() {
        // Fake worker script that immediately prints READY, then echoes a fixed response
        let script = r#"#!/bin/bash
echo "READY"
while IFS= read -r line; do
    echo '[{"start":0.0,"end":1.0,"text":"test"}]'
done
"#;
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("fake_worker.sh");
        std::fs::write(&script_path, script).unwrap();
        std::fs::set_permissions(
            &script_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();

        let worker = WhisperWorker::spawn(script_path.to_str().unwrap())
            .await
            .unwrap();

        let segments = worker.transcribe("/tmp/test.wav".into()).await.unwrap();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "test");
        assert_eq!(segments[0].start, 0.0);
        assert_eq!(segments[0].end, 1.0);
    }

    #[tokio::test]
    async fn worker_handles_error_response() {
        let script = r#"#!/bin/bash
echo "READY"
while IFS= read -r line; do
    echo '{"error":"something went wrong"}'
done
"#;
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("fake_worker_err.sh");
        std::fs::write(&script_path, script).unwrap();
        std::fs::set_permissions(
            &script_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();

        let worker = WhisperWorker::spawn(script_path.to_str().unwrap())
            .await
            .unwrap();

        let result = worker.transcribe("/tmp/test.wav".into()).await;
        let err = result.unwrap_err();
        assert!(err.to_string().contains("something went wrong"));
    }

    #[tokio::test]
    async fn worker_spawn_fails_on_bad_ready() {
        let script = r#"#!/bin/bash
echo "NOT_READY"
"#;
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("bad_ready.sh");
        std::fs::write(&script_path, script).unwrap();
        std::fs::set_permissions(
            &script_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();

        let result = WhisperWorker::spawn(script_path.to_str().unwrap()).await;
        let err = result.unwrap_err();
        assert!(err.to_string().contains("expected READY"));
    }

    #[tokio::test]
    async fn worker_handles_multiple_requests() {
        let script = r#"#!/bin/bash
echo "READY"
count=0
while IFS= read -r line; do
    count=$((count + 1))
    echo "[{\"start\":0.0,\"end\":${count}.0,\"text\":\"segment ${count}\"}]"
done
"#;
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("multi_worker.sh");
        std::fs::write(&script_path, script).unwrap();
        std::fs::set_permissions(
            &script_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();

        let worker = WhisperWorker::spawn(script_path.to_str().unwrap())
            .await
            .unwrap();

        let seg1 = worker.transcribe("/tmp/first.wav".into()).await.unwrap();
        assert_eq!(seg1[0].end, 1.0);
        assert_eq!(seg1[0].text, "segment 1");

        let seg2 = worker.transcribe("/tmp/second.wav".into()).await.unwrap();
        assert_eq!(seg2[0].end, 2.0);
        assert_eq!(seg2[0].text, "segment 2");
    }

    #[tokio::test]
    #[ignore = "requires Python + faster-whisper installed"]
    async fn whisper_worker_integration() {
        let worker = WhisperWorker::spawn("scripts/transcribe.py")
            .await
            .unwrap();
        let result = worker.transcribe("/tmp/test_audio.wav".into()).await;
        println!("{result:?}");
    }
}
