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
    /// The word's canonical written form: Sudachi's *normalized* form, not its
    /// dictionary form.
    ///
    /// The two differ exactly where Japanese spells one word several ways —
    /// いう/言う, できる/出来る, みんな/皆, わかる/分かる — and keying anything on
    /// the dictionary form makes each spelling a separate word with its own
    /// counts and its own status. A reader who judged 言う was then asked to
    /// judge いう, which is the same word wearing kana.
    ///
    /// Normalization subsumes the inflection case too (振っ → 振る), so this is
    /// strictly more canonical than what it replaces.
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
    /// Words the master dictionary lists, for [`SudachiTokenizer::decompose`].
    /// Empty disables it.
    lexicon: HashSet<String>,
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
            lexicon: HashSet::new(),
        })
    }

    /// Teach it which words a dictionary actually lists, enabling compound
    /// decomposition. See [`SudachiTokenizer::decompose`].
    pub fn with_lexicon(mut self, lexicon: HashSet<String>) -> Self {
        self.lexicon = lexicon;
        self
    }

    /// Which spelling of a word to store: the one the master dictionary uses.
    ///
    /// Sudachi normalizes to *its* canonical orthography, and the two
    /// dictionaries do not always agree. する normalizes to 為る, which Sankoku
    /// does not list — so the commonest verb in the language landed as an
    /// unrecognised term with 2,544 encounters, ineligible for triage (the
    /// queue is master-only) and sitting at the top of every unknown-word list.
    ///
    /// Where they disagree the master dictionary wins, because it is the one
    /// that decides what counts as vocabulary at all. Where it lists neither
    /// spelling, normalization stands: it is still the better canonicaliser of
    /// the two (いう → 言う, サーバ → サーバー).
    fn written_form(&self, normalized: &str, dictionary: &str) -> String {
        if self.lexicon.is_empty() || self.lexicon.contains(normalized) {
            return normalized.to_string();
        }
        if self.lexicon.contains(dictionary) {
            return dictionary.to_string();
        }
        normalized.to_string()
    }

    /// Split a compound no dictionary lists into parts that one does.
    ///
    /// Sudachi's own splitting is bounded by its entries: 懲罰房 is a single
    /// entry with no sub-units, so Mode A, B and C all return it whole, and
    /// 懲罰 — a word Sankoku lists, read sixty-one times — was credited to
    /// nothing at all. 医務室 splits fine. Which of the two happens is a
    /// property of Sudachi's dictionary rather than of the language.
    ///
    /// So: longest match from the left, every part a master-dictionary
    /// headword, the whole string consumed, at least two parts. A part must be
    /// two characters or a single kanji — without that, katakana names shred
    /// into whatever one-kana entries the dictionary happens to hold, which
    /// would be inventing vocabulary rather than recovering it.
    ///
    /// Returns `None` when the compound cannot be built from known words,
    /// which leaves it exactly where it was: whole, and visible in the
    /// non-vocabulary tail.
    fn decompose(&self, word: &str) -> Option<Vec<String>> {
        if self.lexicon.is_empty() || self.lexicon.contains(word) {
            return None;
        }
        let chars: Vec<char> = word.chars().collect();
        let acceptable = |part: &str| {
            let n = part.chars().count();
            n >= 2
                || part
                    .chars()
                    .next()
                    .is_some_and(crate::text::kanji::is_kanji)
        };

        let mut parts = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let mut matched = None;
            for end in (i + 1..=chars.len()).rev() {
                let candidate: String = chars[i..end].iter().collect();
                if acceptable(&candidate) && self.lexicon.contains(&candidate) {
                    matched = Some((candidate, end));
                    break;
                }
            }
            let (part, end) = matched?;
            parts.push(part);
            i = end;
        }
        (parts.len() >= 2).then_some(parts)
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
            base_form: self.written_form(m.normalized_form(), m.dictionary_form()),
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
        // Each token that survives the C→B→A pass is offered to `decompose`,
        // which only fires on a compound the master dictionary cannot account
        // for whole but can account for in parts.
        let split_unknown = |tokens: Vec<Token>| -> Result<Vec<Token>, TokenizeError> {
            if self.lexicon.is_empty() {
                return Ok(tokens);
            }
            let mut out = Vec::with_capacity(tokens.len());
            for t in tokens {
                let Some(parts) = self.decompose(&t.base_form) else {
                    out.push(t);
                    continue;
                };
                // Re-analyse each part rather than inventing its reading: the
                // part is a word, so the tokenizer has one for it.
                for part in parts {
                    match tokenizer.tokenize(&part, Mode::C, false) {
                        Ok(ms) if ms.len() == 1 => out.extend(ms.iter().map(&to_token)),
                        // A part that no longer analyses as one morpheme is
                        // not worth guessing at; keep the compound instead.
                        _ => {
                            out.push(t.clone());
                            break;
                        }
                    }
                }
            }
            Ok(out)
        };

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

        split_unknown(tokens)
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
