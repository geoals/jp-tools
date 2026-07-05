//! Fake implementations for frontend development (`JP_TOOLS_FAKE_API=true`).
//! No OCR service, Sudachi dictionary, or Anki needed.

use std::future::Future;
use std::pin::Pin;

use tracing::info;

use jp_core::tokenize::{Token, TokenizeError, Tokenizer};
use jp_mine_core::export::{AnkiExporter, ExportError, ExportSentence};

use crate::services::ocr::{OcrEngine, OcrError};

/// Returns a fixed manga-like line after a short delay.
pub struct FakeOcrEngine;

impl OcrEngine for FakeOcrEngine {
    fn recognize(
        &self,
        _image_bytes: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<String, OcrError>> + Send>> {
        Box::pin(async {
            tokio::time::sleep(std::time::Duration::from_millis(600)).await;
            Ok("お前はもう死んでいる。なに？！".to_string())
        })
    }
}

/// Naive character-level tokenizer that needs no dictionary. CJK ideographs
/// and katakana become content words (clickable in the UI).
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

/// Logs what would be exported and returns success.
pub struct FakeAnkiExporter;

impl AnkiExporter for FakeAnkiExporter {
    fn export_sentences(
        &self,
        sentences: Vec<ExportSentence>,
    ) -> Pin<Box<dyn Future<Output = Result<usize, ExportError>> + Send>> {
        let count = sentences.len();
        Box::pin(async move {
            for es in &sentences {
                info!(
                    source = %es.source,
                    word = ?es.target_word,
                    text = %es.sentence_text,
                    image = ?es.screenshot_path,
                    "[fake] would export sentence to Anki",
                );
            }
            Ok(count)
        })
    }
}
