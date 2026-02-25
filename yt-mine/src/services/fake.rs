//! Fake implementations of external-tool traits for frontend development.
//!
//! Activated by `JP_TOOLS_FAKE_API=true`. No external dependencies needed —
//! no yt-dlp, whisper, ffmpeg, Anki, or UniDic dictionary.

use std::future::Future;
use std::pin::Pin;

use tracing::info;

use crate::models::TranscriptSegment;
use crate::services::download::{AudioDownloader, DownloadError, DownloadResult};
use crate::services::export::{AnkiExporter, ExportError, ExportSentence};
use crate::services::llm::{LlmDefiner, LlmError};
use crate::services::media::{MediaError, MediaExtractor};
use crate::services::tokenize::{Token, TokenizeError, Tokenizer};
use crate::services::transcribe::{TranscribeError, Transcriber};

/// Returns hardcoded download paths after a short delay (so the
/// "Downloading..." state is visible during htmx polling).
pub struct FakeDownloader;

impl AudioDownloader for FakeDownloader {
    fn download(
        &self,
        _url: String,
        output_dir: String,
    ) -> Pin<Box<dyn Future<Output = Result<DownloadResult, DownloadError>> + Send>> {
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            let audio_path = format!("{output_dir}/fake_api.wav");
            let video_path = format!("{output_dir}/fake_api.mp4");

            // Create placeholder files so paths stored in the DB actually exist.
            for path in [&audio_path, &video_path] {
                tokio::fs::write(path, b"fake")
                    .await
                    .map_err(|e| DownloadError::Failed(format!("failed to write {path}: {e}")))?;
            }

            Ok(DownloadResult {
                audio_path,
                video_path,
                video_title: "[Dev] 日本語の勉強法".into(),
            })
        })
    }
}

/// Returns hardcoded Japanese transcript segments after a short delay (so the
/// "Transcribing..." state is visible during htmx polling).
pub struct FakeTranscriber;

impl Transcriber for FakeTranscriber {
    fn transcribe(
        &self,
        _audio_path: String,
        on_progress: Option<crate::services::transcribe::ProgressCallback>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<TranscriptSegment>, TranscribeError>> + Send>> {
        Box::pin(async move {
            let segments = fake_segments();
            for (i, seg) in segments.iter().enumerate() {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                if let Some(cb) = &on_progress {
                    cb(seg.clone(), i + 1);
                }
            }
            Ok(segments)
        })
    }
}

fn fake_segments() -> Vec<TranscriptSegment> {
    vec![
        TranscriptSegment { start: 0.0,  end: 3.5,  text: "皆さん、こんにちは。今日は日本語の勉強法についてお話しします。".into() },
        TranscriptSegment { start: 3.5,  end: 7.2,  text: "まず最初に、毎日少しずつ練習することが大切です。".into() },
        TranscriptSegment { start: 7.2,  end: 11.0, text: "新しい単語を覚えるときは、文脈の中で学ぶのが効果的です。".into() },
        TranscriptSegment { start: 11.0, end: 15.3, text: "例えば、好きなアニメや映画を日本語で見ることをお勧めします。".into() },
        TranscriptSegment { start: 15.3, end: 19.8, text: "字幕を使って、聞き取れなかった部分を確認しましょう。".into() },
        TranscriptSegment { start: 19.8, end: 24.1, text: "文法は難しいと感じるかもしれませんが、パターンを見つけると楽になります。".into() },
        TranscriptSegment { start: 24.1, end: 28.5, text: "漢字の勉強も忘れないでください。読めると世界が広がります。".into() },
        TranscriptSegment { start: 28.5, end: 32.0, text: "最後に、間違いを恐れずにたくさん話してみてください。".into() },
        TranscriptSegment { start: 32.0, end: 36.2, text: "日本人の友達を作ると、会話の練習になりますよ。".into() },
        TranscriptSegment { start: 36.2, end: 40.0, text: "それでは、頑張ってください。応援しています。".into() },
    ]
}

/// No-op screenshot extraction; writes a placeholder file for audio clips
/// so the audio endpoint returns 200.
pub struct FakeMediaExtractor;

impl MediaExtractor for FakeMediaExtractor {
    fn extract_screenshot(
        &self,
        _video_path: &str,
        _timestamp_secs: f64,
        _output_path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), MediaError>> + Send>> {
        Box::pin(async { Ok(()) })
    }

    fn extract_audio_clip(
        &self,
        _audio_path: &str,
        _start_secs: f64,
        _end_secs: f64,
        output_path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), MediaError>> + Send>> {
        let output_path = output_path.to_owned();
        Box::pin(async move {
            tokio::fs::write(&output_path, b"fake-audio")
                .await
                .map_err(|e| MediaError::Failed(format!("failed to write fake audio: {e}")))?;
            Ok(())
        })
    }
}

/// Naive character-level tokenizer that needs no dictionary. Splits text
/// into individual characters: CJK ideographs and katakana are marked as
/// content words (clickable in the UI), everything else is not.
pub struct FakeTokenizer;

impl Tokenizer for FakeTokenizer {
    fn tokenize(&self, text: &str) -> Result<Vec<Token>, TokenizeError> {
        Ok(text
            .chars()
            .map(|c| {
                let s = c.to_string();
                let is_cjk = matches!(c,
                    '\u{4E00}'..='\u{9FFF}'   // CJK Unified Ideographs
                    | '\u{30A0}'..='\u{30FF}'  // Katakana
                );
                Token {
                    surface: s.clone(),
                    base_form: s,
                    reading: "*".into(),
                    pos: if is_cjk { "名詞" } else { "記号" }.into(),
                }
            })
            .collect())
    }
}

/// Returns a placeholder LLM definition string.
pub struct FakeLlmDefiner;

impl LlmDefiner for FakeLlmDefiner {
    fn define(
        &self,
        word: &str,
        _sentence_context: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, LlmError>> + Send>> {
        let word = word.to_owned();
        Box::pin(async move { Ok(format!("[LLM definition for {word}]")) })
    }
}

/// Logs what would be exported and returns success.
pub struct FakeAnkiExporter;

impl AnkiExporter for FakeAnkiExporter {
    fn export_sentences(
        &self,
        sentences: Vec<ExportSentence>,
        source: String,
    ) -> Pin<Box<dyn Future<Output = Result<usize, ExportError>> + Send>> {
        let count = sentences.len();
        Box::pin(async move {
            for es in &sentences {
                info!(
                    source = %source,
                    word = ?es.target_word,
                    text = %es.sentence.text,
                    "[fake] would export sentence to Anki",
                );
            }
            Ok(count)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_downloader_creates_placeholder_files() {
        let dir = tempfile::tempdir().unwrap();
        let dl = FakeDownloader;
        let result = dl.download("https://youtu.be/test".into(), dir.path().to_str().unwrap().into()).await.unwrap();
        assert!(std::path::Path::new(&result.audio_path).exists());
        assert!(std::path::Path::new(&result.video_path).exists());
        assert!(result.video_title.contains("Dev"));
    }

    #[tokio::test]
    async fn fake_transcriber_returns_non_empty_segments() {
        let t = FakeTranscriber;
        let segments = t.transcribe("/tmp/audio.wav".into(), None).await.unwrap();
        assert!(!segments.is_empty());
        // Timestamps should be increasing
        for window in segments.windows(2) {
            assert!(window[1].start >= window[0].end);
        }
    }

    #[tokio::test]
    async fn fake_media_extractor_writes_audio_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.mp3");
        let extractor = FakeMediaExtractor;

        extractor
            .extract_audio_clip("in.wav", 0.0, 1.0, path.to_str().unwrap())
            .await
            .unwrap();

        assert!(path.exists());
    }

    #[test]
    fn fake_tokenizer_marks_kanji_as_content_words() {
        use crate::services::tokenize::is_content_word;

        let t = FakeTokenizer;
        let tokens = t.tokenize("漢字とカタカナ。").unwrap();

        // Kanji and katakana should be content words (clickable)
        let kanji: Vec<_> = tokens.iter().filter(|t| is_content_word(&t.pos)).collect();
        assert!(kanji.iter().any(|t| t.surface == "漢"));
        assert!(kanji.iter().any(|t| t.surface == "カ"));

        // Hiragana and punctuation should not
        let non_content: Vec<_> = tokens.iter().filter(|t| !is_content_word(&t.pos)).collect();
        assert!(non_content.iter().any(|t| t.surface == "と"));
        assert!(non_content.iter().any(|t| t.surface == "。"));
    }

    #[tokio::test]
    async fn fake_llm_definer_returns_placeholder() {
        let definer = FakeLlmDefiner;
        let result = definer.define("食べる", "毎日ラーメンを食べる").await.unwrap();
        assert_eq!(result, "[LLM definition for 食べる]");
    }

    #[tokio::test]
    async fn fake_anki_exporter_returns_correct_count() {
        use crate::models::Sentence;

        let exporter = FakeAnkiExporter;
        let sentences = vec![
            ExportSentence {
                sentence: Sentence {
                    id: 1, job_id: 1, text: "test".into(),
                    start_time: 0.0, end_time: 1.0, created_at: "0".into(),
                },
                screenshot_path: None, audio_clip_path: None,
                target_word: Some("test".into()), definition: None,
                vocab_furigana: None, vocab_pitch_num: None, sentence_html: None,
                llm_definition: None,
            },
            ExportSentence {
                sentence: Sentence {
                    id: 2, job_id: 1, text: "test2".into(),
                    start_time: 1.0, end_time: 2.0, created_at: "0".into(),
                },
                screenshot_path: None, audio_clip_path: None,
                target_word: None, definition: None,
                vocab_furigana: None, vocab_pitch_num: None, sentence_html: None,
                llm_definition: None,
            },
        ];

        let count = exporter.export_sentences(sentences, "test".into()).await.unwrap();
        assert_eq!(count, 2);
    }
}
