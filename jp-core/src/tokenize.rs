use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use sudachi::analysis::stateless_tokenizer::DictionaryAccess;
use sudachi::analysis::stateless_tokenizer::StatelessTokenizer;
use sudachi::analysis::{Mode, Tokenize};
use sudachi::config::Config;
use sudachi::dic::dictionary::JapaneseDictionary;
use sudachi::dic::subset::InfoSubset;
use sudachi::dic::word_id::WordId;
use sudachi::prelude::Morpheme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub surface: String,
    pub base_form: String,
    pub reading: String,
    /// Top-level part of speech (名詞, 動詞, …).
    pub pos: String,
    /// Whether Sudachi calls this a proper noun (固有名詞 — the second field of
    /// its part-of-speech tuple, which we otherwise discard).
    ///
    /// Kept because a name is not vocabulary: a VN's cast are the commonest
    /// "unknown words" in it and learning them is not learning Japanese.
    pub proper_noun: bool,
}

#[cfg_attr(any(test, feature = "test-support"), mockall::automock)]
pub trait Tokenizer: Send + Sync {
    fn tokenize(&self, text: &str) -> Result<Vec<Token>, TokenizeError>;
}

#[derive(Debug, thiserror::Error)]
pub enum TokenizeError {
    #[error("tokenization failed: {0}")]
    Failed(String),
}

pub struct SudachiTokenizer {
    dict: Arc<JapaneseDictionary>,
    headwords: HashSet<String>,
}

impl SudachiTokenizer {
    pub fn new(dict_path: &Path, headwords: HashSet<String>) -> Result<Self, TokenizeError> {
        let abs_path = std::fs::canonicalize(dict_path).map_err(|e| {
            TokenizeError::Failed(format!(
                "dictionary not found at {}: {e}",
                dict_path.display()
            ))
        })?;
        let config = Config::new(None, None, Some(abs_path))
            .map_err(|e| TokenizeError::Failed(format!("failed to load Sudachi config: {e}")))?;
        let dict = JapaneseDictionary::from_cfg(&config).map_err(|e| {
            TokenizeError::Failed(format!("failed to load Sudachi dictionary: {e}"))
        })?;
        Ok(Self {
            dict: Arc::new(dict),
            headwords,
        })
    }
}

impl SudachiTokenizer {
    /// The reading of the morpheme's **dictionary form**, not of its surface.
    ///
    /// `Morpheme::reading_form` is the reading of the text as it appeared:
    /// 振って gives フッ, 知らない gives シラ. Pairing that with
    /// `dictionary_form` — which is the lemma, 振る — produces a term nobody
    /// ever wrote, 振る/ふっ, and worse, splits one word across as many ledger
    /// rows as it has inflected stems (知る appeared as しる, しら and しっ,
    /// each with its own counts and its own status).
    ///
    /// Sudachi already knows the answer: a conjugated entry carries the word id
    /// of its dictionary form, so the reading is one lexicon lookup away. It
    /// resolves that id itself for the *surface* of the dictionary form and
    /// stops there, which is why this has to ask for the reading separately.
    ///
    /// Falls back to the surface reading whenever there is no dictionary form
    /// to consult (out-of-vocabulary morphemes, or an entry that is already its
    /// own lemma) — for those the two are the same thing anyway.
    fn dictionary_form_reading<T: DictionaryAccess>(&self, m: &Morpheme<'_, T>) -> String {
        let surface_reading = || m.reading_form().to_string();
        let wid = m.word_id();
        if wid.is_oov() {
            return surface_reading();
        }
        let lemma_id = m.get_word_info().dictionary_form_word_id();
        if lemma_id < 0 || lemma_id as u32 == wid.word() {
            return surface_reading();
        }
        self.dict
            .lexicon()
            .get_word_info_subset(
                WordId::new(wid.dic(), lemma_id as u32),
                InfoSubset::READING_FORM,
            )
            .map(|wi| wi.reading_form().to_string())
            .unwrap_or_else(|_| surface_reading())
    }
}

impl Tokenizer for SudachiTokenizer {
    fn tokenize(&self, text: &str) -> Result<Vec<Token>, TokenizeError> {
        let tokenizer = StatelessTokenizer::new(&self.dict);
        let to_token = |m: sudachi::prelude::Morpheme<'_, _>| Token {
            surface: m.surface().to_string(),
            base_form: m.dictionary_form().to_string(),
            reading: self.dictionary_form_reading(&m),
            pos: m.part_of_speech()[0].clone(),
            // [0] is the top-level class, [1] the subclass: 名詞,固有名詞,人名.
            proper_noun: m.part_of_speech().get(1).is_some_and(|p| p == "固有名詞"),
        };

        if self.headwords.is_empty() {
            // No dictionaries loaded — Mode B (current behavior)
            let morphemes = tokenizer
                .tokenize(text, Mode::B, false)
                .map_err(|e| TokenizeError::Failed(e.to_string()))?;
            return Ok(morphemes.iter().map(&to_token).collect());
        }

        // Dictionary-validated splitting: C → B → A.
        // Keep tokens that exist as dictionary headwords. Split unknown
        // compounds progressively (C→B→A) until sub-tokens are recognized
        // or we reach the finest granularity.
        let morphemes = tokenizer
            .tokenize(text, Mode::C, false)
            .map_err(|e| TokenizeError::Failed(e.to_string()))?;

        let err = |e: sudachi::error::SudachiError| TokenizeError::Failed(e.to_string());
        let mut buf_b = morphemes.empty_clone();
        let mut buf_a = morphemes.empty_clone();
        let mut tokens = Vec::new();

        for m in morphemes.iter() {
            if self.headwords.contains(m.dictionary_form()) {
                tokens.push(to_token(m));
                continue;
            }

            buf_b.clear();
            if !m.split_into(Mode::B, &mut buf_b).map_err(&err)? {
                // Mode B didn't split — try Mode A directly
                buf_a.clear();
                if m.split_into(Mode::A, &mut buf_a).map_err(&err)? {
                    tokens.extend(buf_a.iter().map(&to_token));
                } else {
                    tokens.push(to_token(m));
                }
                continue;
            }

            // Mode B split — check each sub-token
            for sub in buf_b.iter() {
                if self.headwords.contains(sub.dictionary_form()) {
                    tokens.push(to_token(sub));
                } else {
                    buf_a.clear();
                    if sub.split_into(Mode::A, &mut buf_a).map_err(&err)? {
                        tokens.extend(buf_a.iter().map(&to_token));
                    } else {
                        tokens.push(to_token(sub));
                    }
                }
            }
        }

        Ok(tokens)
    }
}

/// Returns true if the part-of-speech tag represents a content word
/// (noun, verb, adjective, adjectival noun, adverb).
pub fn is_content_word(pos: &str) -> bool {
    matches!(pos, "名詞" | "動詞" | "形容詞" | "形状詞" | "副詞")
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
    #[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
    fn sudachi_tokenizer_produces_tokens() {
        let dict_path = std::env::var("JP_TOOLS_SUDACHI_DICT_PATH")
            .expect("JP_TOOLS_SUDACHI_DICT_PATH must be set");
        let tokenizer = SudachiTokenizer::new(Path::new(&dict_path), HashSet::new()).unwrap();
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

    /// A VN's cast are the commonest "unknown words" in it, and none of them
    /// is vocabulary. Sudachi's subclass is the signal; nothing else available
    /// here distinguishes them.
    #[test]
    #[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
    fn proper_nouns_are_marked_and_ordinary_words_are_not() {
        let dict_path = std::env::var("JP_TOOLS_SUDACHI_DICT_PATH")
            .expect("JP_TOOLS_SUDACHI_DICT_PATH must be set");
        let tokenizer = SudachiTokenizer::new(Path::new(&dict_path), HashSet::new()).unwrap();

        let tokens = tokenizer.tokenize("東京で田中さんが本を読む").unwrap();
        let named = |s: &str| tokens.iter().find(|t| t.surface == s).unwrap().proper_noun;
        assert!(named("東京"), "a place name");
        assert!(named("田中"), "a person's name");
        assert!(!named("本"), "an ordinary noun");
        assert!(!named("読む"), "a verb");
    }

    /// The reading must belong to the same form as the headword beside it.
    ///
    /// Sudachi's `reading_form` is the *surface* reading, so a conjugated verb
    /// used to produce (振る, フッ) — a pair nobody ever wrote — and split one
    /// verb across a ledger row per inflected stem.
    #[test]
    #[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
    fn a_conjugated_verb_carries_its_dictionary_forms_reading() {
        let dict_path = std::env::var("JP_TOOLS_SUDACHI_DICT_PATH")
            .expect("JP_TOOLS_SUDACHI_DICT_PATH must be set");
        let tokenizer = SudachiTokenizer::new(Path::new(&dict_path), HashSet::new()).unwrap();

        for (text, surface, base, reading) in [
            ("手を振って", "振っ", "振る", "フル"),
            ("何も知らない", "知ら", "知る", "シル"),
            ("部屋に入った", "入っ", "入る", "ハイル"),
            ("考えていた", "考え", "考える", "カンガエル"),
            // Already its own lemma: nothing to resolve, and nothing changes.
            ("東京に行く", "行く", "行く", "イク"),
        ] {
            let tokens = tokenizer.tokenize(text).unwrap();
            let t = tokens
                .iter()
                .find(|t| t.surface == surface)
                .unwrap_or_else(|| panic!("{surface} not found in {text}: {tokens:?}"));
            assert_eq!(t.base_form, base, "base form of {surface}");
            assert_eq!(t.reading, reading, "reading of {surface} in {text}");
        }
    }
}
