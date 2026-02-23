use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub surface: String,
    pub base_form: String,
    pub reading: String,
    pub pos: String,
}

#[cfg_attr(test, mockall::automock)]
pub trait Tokenizer: Send + Sync {
    fn tokenize(&self, text: &str) -> Result<Vec<Token>, TokenizeError>;
}

#[derive(Debug, thiserror::Error)]
pub enum TokenizeError {
    #[error("tokenization failed: {0}")]
    Failed(String),
}

/// Wraps lindera's tokenizer in a Mutex since it requires `&mut self`
/// for tokenization (not Sync). Sub-millisecond per call, so contention
/// is not a concern.
pub struct LinderaTokenizer {
    inner: Mutex<lindera::tokenizer::Tokenizer>,
}

impl LinderaTokenizer {
    pub fn new() -> Result<Self, TokenizeError> {
        let dictionary = lindera::dictionary::load_dictionary("embedded://unidic")
            .map_err(|e| TokenizeError::Failed(format!("failed to load UniDic: {e}")))?;
        let segmenter =
            lindera::segmenter::Segmenter::new(lindera::mode::Mode::Normal, dictionary, None);
        let tokenizer = lindera::tokenizer::Tokenizer::new(segmenter);
        Ok(Self {
            inner: Mutex::new(tokenizer),
        })
    }
}

impl Tokenizer for LinderaTokenizer {
    fn tokenize(&self, text: &str) -> Result<Vec<Token>, TokenizeError> {
        let tokenizer = self
            .inner
            .lock()
            .map_err(|e| TokenizeError::Failed(format!("lock poisoned: {e}")))?;
        let mut tokens = tokenizer
            .tokenize(text)
            .map_err(|e| TokenizeError::Failed(e.to_string()))?;

        Ok(tokens
            .iter_mut()
            .map(|t| {
                let surface = t.surface.to_string();
                let pos = t
                    .get("part_of_speech")
                    .unwrap_or("*")
                    .to_string();
                let base_form = t
                    .get("orthographic_base_form")
                    .unwrap_or(&surface)
                    .to_string();
                let reading = t
                    .get("reading")
                    .unwrap_or("*")
                    .to_string();
                Token {
                    surface,
                    base_form,
                    reading,
                    pos,
                }
            })
            .collect())
    }
}

/// Returns true if the part-of-speech tag represents a content word
/// (noun, verb, adjective, adjectival noun, adverb).
pub fn is_content_word(pos: &str) -> bool {
    matches!(
        pos,
        "名詞" | "動詞" | "形容詞" | "形状詞" | "副詞"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_content_word_matches_nouns() {
        assert!(is_content_word("名詞"));
    }

    #[test]
    fn is_content_word_matches_verbs() {
        assert!(is_content_word("動詞"));
    }

    #[test]
    fn is_content_word_matches_adjectives() {
        assert!(is_content_word("形容詞"));
    }

    #[test]
    fn is_content_word_matches_adjectival_nouns() {
        assert!(is_content_word("形状詞"));
    }

    #[test]
    fn is_content_word_matches_adverbs() {
        assert!(is_content_word("副詞"));
    }

    #[test]
    fn is_content_word_rejects_particles() {
        assert!(!is_content_word("助詞"));
    }

    #[test]
    fn is_content_word_rejects_auxiliary_verbs() {
        assert!(!is_content_word("助動詞"));
    }

    #[test]
    fn is_content_word_rejects_punctuation() {
        assert!(!is_content_word("補助記号"));
    }

    #[test]
    fn is_content_word_rejects_empty_string() {
        assert!(!is_content_word(""));
    }

    #[test]
    #[ignore = "requires lindera UniDic (embedded, slow to init)"]
    fn lindera_tokenizer_produces_tokens() {
        let tokenizer = LinderaTokenizer::new().unwrap();
        let tokens = tokenizer.tokenize("東京に行く").unwrap();

        assert!(!tokens.is_empty());

        // 東京 should be a noun
        let tokyo = tokens.iter().find(|t| t.surface == "東京").unwrap();
        assert_eq!(tokyo.pos, "名詞");
        assert!(is_content_word(&tokyo.pos));

        // に should be a particle
        let ni = tokens.iter().find(|t| t.surface == "に").unwrap();
        assert!(!is_content_word(&ni.pos));

        // 行く should be a verb with base form 行く
        let iku = tokens.iter().find(|t| t.surface == "行く").unwrap();
        assert_eq!(iku.pos, "動詞");
        assert_eq!(iku.base_form, "行く");
    }
}
