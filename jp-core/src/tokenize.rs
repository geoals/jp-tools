use std::collections::{HashMap, HashSet};
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
    /// Master headword, keyed by its reading — for the kana half of
    /// [`SudachiTokenizer::recompose`]. Only readings that name exactly one
    /// headword are in here. Empty disables reading-matched recomposition.
    by_reading: HashMap<String, String>,
    /// A master headword's own reading, so a recomposed token carries the
    /// reading the ledger will key it on rather than one built from parts.
    term_reading: HashMap<String, String>,
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
            by_reading: HashMap::new(),
            term_reading: HashMap::new(),
        })
    }

    /// Teach it which words a dictionary actually lists, enabling compound
    /// decomposition. See [`SudachiTokenizer::decompose`].
    pub fn with_lexicon(mut self, lexicon: HashSet<String>) -> Self {
        self.lexicon = lexicon;
        self
    }

    /// Teach it how the master dictionary *reads* its headwords, enabling the
    /// kana half of [`SudachiTokenizer::recompose`].
    ///
    /// Separate from [`with_lexicon`](Self::with_lexicon) because it is the
    /// weaker signal and a caller may reasonably want spelling-matched
    /// recomposition without it. A reading naming more than one headword is
    /// dropped rather than arbitrated: おこす is 起こす and 興す, and merging
    /// two tokens into a guess about which is worse than leaving them apart.
    pub fn with_master_readings(mut self, entries: &[(String, String)]) -> Self {
        let mut ambiguous: HashSet<String> = HashSet::new();
        for (term, reading) in entries {
            if reading.is_empty() {
                continue;
            }
            let reading = crate::text::kana::to_hiragana(reading);
            self.term_reading
                .entry(term.clone())
                .or_insert_with(|| reading.clone());
            match self.by_reading.get(&reading) {
                Some(seen) if seen != term => {
                    ambiguous.insert(reading);
                }
                Some(_) => {}
                None => {
                    self.by_reading.insert(reading, term.clone());
                }
            }
        }
        for reading in ambiguous {
            self.by_reading.remove(&reading);
        }
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
    /// headword, the whole string consumed, at least two parts.
    ///
    /// A one-character part must be kanji. Kana of either alphabet shreds:
    /// ミリア becomes ミ + リ + ア, and 楽しみ becomes 楽し + み — a dictionary
    /// lists み as a noun, so the pieces pass every test except sense. Allowing
    /// hiragana produced み ×69 and め ×38 out of nothing, against 凛と's two
    /// sightings of 凛 that it recovered. A compound ending in a bare kana is
    /// therefore left whole, and lands in the non-vocabulary tail if no
    /// dictionary claims it.
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

    /// Join adjacent tokens that the master dictionary lists as one word.
    ///
    /// The mirror of [`decompose`](Self::decompose), and the case neither it
    /// nor the C→B→A pass could reach. Both of those only ever *split*: the
    /// wordhood gate can reject a token into smaller pieces and never build a
    /// larger one, so a compound Sudachi's own lexicon lacks is gone before any
    /// of our logic sees it. しゃくりあげる is not a Sudachi entry, so Mode C
    /// hands back しゃくり + あげ and the ledger credited しゃくる and 上げる —
    /// while 噦り上げる, which Sankoku lists, was never met once. It is not a
    /// rare shape: over the first 14,519 tracked lines, 570 distinct
    /// master-dictionary compounds were being shredded this way across 1,663
    /// occurrences (落ち着く, 思い出す, 立ち上がる, 振り返る, 巻き込む…), and
    /// 317 of the ledger rows for them sat at zero encounters while their parts
    /// collected the sightings.
    ///
    /// Longest match first, left to right, at most [`MAX_COMPOUND_PARTS`] parts.
    /// A run is joined on either of two signals:
    ///
    /// - **spelling** — the parts as written spell a master headword
    ///   (振り + 返る → 振り返る). Matching the dictionary's literal headword is
    ///   strong evidence on its own.
    /// - **reading** — the parts read as a master headword
    ///   (しゃくり + あげる → しゃくりあげる → 噦り上げる). Needed because the
    ///   text writes in kana what the dictionary spells in kanji, and this is
    ///   the same gap the non-word gate had before it learned to match
    ///   readings.
    ///
    /// The reading signal is the weaker one and is fenced in accordingly: every
    /// part must be a verb, and every part but the last must already be kana.
    /// Without that fence it merges そう + する into 相する and こと + し into
    /// 今年 — adverb-plus-verb and noun-plus-verb runs that happen to *read*
    /// like a listed word. Verb + verb is the shape the defect actually takes.
    ///
    /// Three guards apply to both signals. Every part must be a content word,
    /// or ていた becomes 訂 + 板. **No part may be a proper noun** — the same
    /// rule `decompose` needs in the other direction, and for the same reason:
    /// a general dictionary lists no cast member, so a name beside a noun is a
    /// compound waiting to be invented. And the result must be at least three
    /// characters, which is what keeps two-character kana homographs out.
    ///
    /// The joined token takes the master's own spelling and reading, so the
    /// ledger keys it the way every other pass would.
    fn recompose(&self, tokens: Vec<Token>) -> Vec<Token> {
        if self.lexicon.is_empty() && self.by_reading.is_empty() {
            return tokens;
        }
        let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
        let mut i = 0;
        while i < tokens.len() {
            let longest = MAX_COMPOUND_PARTS.min(tokens.len() - i);
            let joined = (2..=longest)
                .rev()
                .find_map(|n| self.join_run(&tokens[i..i + n]));
            match joined {
                Some(token) => {
                    i += token.parts;
                    out.push(token.token);
                }
                None => {
                    out.push(tokens[i].clone());
                    i += 1;
                }
            }
        }
        out
    }

    /// One candidate run, joined or refused. See [`recompose`](Self::recompose)
    /// for the rules; this is only their transcription.
    fn join_run(&self, run: &[Token]) -> Option<Joined> {
        if run
            .iter()
            .any(|t| t.surface.is_empty() || t.proper_noun || !is_content_word(&t.pos))
        {
            return None;
        }
        let (last, head) = run.split_last()?;

        let written: String = head
            .iter()
            .map(|t| t.surface.as_str())
            .chain(std::iter::once(last.base_form.as_str()))
            .collect();
        let term = if self.lexicon.contains(&written) {
            Some(written)
        } else if run.iter().all(|t| t.pos == "動詞")
            && head
                .iter()
                .all(|t| crate::text::kana::is_all_kana(&t.surface))
        {
            let read: String = head
                .iter()
                .map(|t| crate::text::kana::to_hiragana(&t.surface))
                .chain(std::iter::once(crate::text::kana::to_hiragana(
                    &last.reading,
                )))
                .collect();
            self.by_reading.get(&read).cloned()
        } else {
            None
        }?;

        if term.chars().count() < 3 {
            return None;
        }
        Some(Joined {
            parts: run.len(),
            token: Token {
                surface: run.iter().map(|t| t.surface.as_str()).collect(),
                reading: self
                    .term_reading
                    .get(&term)
                    .cloned()
                    .unwrap_or_else(|| last.reading.clone()),
                base_form: term,
                pos: last.pos.clone(),
                proper_noun: false,
            },
        })
    }
}

/// How many tokens [`SudachiTokenizer::recompose`] will join at once. Three
/// covers 申し訳ない and もう一度; beyond that the candidates stop being
/// compounds and start being phrases, which a vocabulary ledger does not want.
const MAX_COMPOUND_PARTS: usize = 3;

/// A joined run, and how many tokens it consumed.
struct Joined {
    token: Token,
    parts: usize,
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
            // Still recomposed: an empty deck is a reason to skip the wordhood
            // gate, not a reason to shred every compound the master lists.
            return Ok(self.recompose(morphemes.iter().map(&to_token).collect()));
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
                // Never take a name apart. A general dictionary lists no place
                // or surname, so every one of them looks like an unlistable
                // compound: 東京 became 東 + 京, 間宮 became 間 + 宮, and the
                // parts are ordinary nouns that the name filter downstream has
                // no way to recognise — it can only see what a token *is*, and
                // 京 is a word. Twenty-two sightings of Tokyo turned into
                // twenty-two of "capital".
                if t.proper_noun {
                    out.push(t);
                    continue;
                }
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

        // Recomposition goes last, over the finished stream: it has to see the
        // tokens the splitting passes actually produced, since those are what
        // shredded the compound in the first place.
        Ok(self.recompose(split_unknown(tokens)?))
    }
}

/// Returns true if the part-of-speech tag represents a content word
/// (noun, verb, adjective, adjectival noun, adverb).
pub fn is_content_word(pos: &str) -> bool {
    matches!(pos, "名詞" | "動詞" | "形容詞" | "形状詞" | "副詞")
}

/// Sudachi's affix classes — the tags [`MasterWords`] arbitrates.
fn is_affix(pos: &str) -> bool {
    matches!(pos, "接尾辞" | "接頭辞")
}

/// The master dictionary, asked the only question the affix rule needs: does it
/// list this `(headword, reading)`?
///
/// Keyed the way `knowledge::vocabulary::Term` is keyed, and it has to be: a
/// kana-only headword stores no reading there (ちゃん is one fact, not two), so
/// asking for a pair would never match one. The reading is folded to hiragana
/// for the same reason — Sudachi emits katakana, the dictionaries hold
/// hiragana.
pub struct MasterWords {
    headwords: HashSet<String>,
    pairs: HashSet<(String, String)>,
}

impl MasterWords {
    pub fn new(headwords: HashSet<String>, entries: &[(String, String)]) -> MasterWords {
        let pairs = entries
            .iter()
            .map(|(term, reading)| (term.clone(), crate::text::kana::to_hiragana(reading)))
            .collect();
        MasterWords { headwords, pairs }
    }

    pub fn lists(&self, headword: &str, reading: &str) -> bool {
        if crate::text::kana::is_all_kana(headword) {
            return self.headwords.contains(headword);
        }
        self.pairs.contains(&(
            headword.to_string(),
            crate::text::kana::to_hiragana(reading),
        ))
    }
}

/// Whether a token is one the ledger counts as a word.
///
/// Content words, plus **an affix the master dictionary lists under the reading
/// it was used with**. That second clause is not a special case, it is the
/// decomposition rule finishing its sentence: 私達 is not a Sankoku entry, so it
/// arrives as 私 + 達 — and 達/たち *is* a Sankoku entry, so throwing it away
/// credited half the compound to nothing, exactly as 懲罰房 did. Sudachi tags
/// the trailing part 接尾辞 rather than 名詞, which is the only reason the
/// content-word gate ever saw a difference between the two halves.
///
/// The pair test is the whole fence, and it is the same authority every other
/// decision here answers to. It admits 達/たち, 御/お, 的/てき, 鬼/き; it refuses
/// げ, ぷ, さん/さーん and 日/じつ — 40 terms over 198 occurrences in the first
/// 16,325 lines — without a stoplist to maintain or a shape to guess at.
pub fn counts_as_word(t: &Token, master: &MasterWords) -> bool {
    is_content_word(&t.pos) || (is_affix(&t.pos) && master.lists(&t.base_form, &t.reading))
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

    fn affix(surface: &str, base: &str, reading: &str, pos: &str) -> Token {
        Token {
            surface: surface.to_string(),
            base_form: base.to_string(),
            reading: reading.to_string(),
            pos: pos.to_string(),
            proper_noun: false,
        }
    }

    /// Sankoku as far as these tests are concerned: 達/たち, 鬼/き, ちゃん.
    fn master() -> MasterWords {
        MasterWords::new(
            ["達", "鬼", "ちゃん"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            &[
                ("達".to_string(), "たち".to_string()),
                ("鬼".to_string(), "き".to_string()),
                ("鬼".to_string(), "おに".to_string()),
            ],
        )
    }

    #[test]
    fn a_listed_suffix_counts_as_a_word() {
        // 私達 is not a Sankoku entry, so it arrives as 私 + 達 — and 達/たち is
        // one, so the suffix half has to be credited too.
        let m = master();
        assert!(counts_as_word(&affix("達", "達", "タチ", "接尾辞"), &m));
        assert!(counts_as_word(&affix("鬼", "鬼", "キ", "接尾辞"), &m));
    }

    #[test]
    fn a_kana_suffix_matches_on_the_headword_alone() {
        // Term::new stores no reading for a kana headword, so a pair lookup
        // would never match ちゃん however it was written.
        assert!(counts_as_word(
            &affix("ちゃん", "ちゃん", "チャン", "接尾辞"),
            &master()
        ));
    }

    #[test]
    fn an_unlisted_suffix_is_still_dropped() {
        // げ, ぷ, さん/さーん: real Sudachi output, no dictionary behind it.
        assert!(!counts_as_word(
            &affix("げ", "げ", "ゲ", "接尾辞"),
            &master()
        ));
    }

    #[test]
    fn a_listed_headword_under_the_wrong_reading_is_dropped() {
        // 鬼 is listed as き and おに and nothing else; a third reading is the
        // tokenizer having produced something the dictionary does not claim.
        assert!(!counts_as_word(
            &affix("鬼", "鬼", "シコ", "接尾辞"),
            &master()
        ));
    }

    #[test]
    fn the_affix_rule_never_admits_a_particle() {
        // The gate is content-word OR *affix*; a particle the master happens to
        // list as a headword must not slip through it.
        let m = MasterWords::new(
            ["は".to_string()].into_iter().collect(),
            &[("は".to_string(), "は".to_string())],
        );
        assert!(!counts_as_word(&affix("は", "は", "ハ", "助詞"), &m));
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

    /// A compound the master dictionary cannot account for whole is taken
    /// apart into words it does list — including across a one-character
    /// hiragana tail, which is where 凛 was being lost.
    #[test]
    #[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
    fn compounds_decompose_into_dictionary_words_but_names_do_not() {
        let dict_path = std::env::var("JP_TOOLS_SUDACHI_DICT_PATH")
            .expect("JP_TOOLS_SUDACHI_DICT_PATH must be set");
        // Stands in for the master dictionary's headwords.
        let lexicon: HashSet<String> = ["凛", "と", "白蓮", "華", "東", "京", "ミ", "リ", "ア"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let tokenizer = SudachiTokenizer::new(Path::new(&dict_path), HashSet::from(["x".into()]))
            .unwrap()
            .with_lexicon(lexicon);
        let bases = |text: &str| {
            tokenizer
                .tokenize(text)
                .unwrap()
                .into_iter()
                .map(|t| t.base_form)
                .collect::<Vec<_>>()
        };

        // Sudachi holds this whole — no mode splits it — and calls it an
        // ordinary noun, so it is taken apart into words the dictionary lists.
        assert_eq!(bases("白蓮華"), vec!["白蓮", "華"]);
        // A bare kana tail is not a part: 楽しみ would otherwise become
        // 楽し + み, since a dictionary does list み as a noun.
        assert_eq!(bases("凛とした")[0], "凛と");
        // Katakana is excluded, or a name becomes three "words".
        assert_eq!(bases("ミリア").len(), 1, "a name is not a compound");
        // And neither is a place. A general dictionary lists no place names,
        // so 東京 looks exactly like an unlistable compound of two words it
        // does list — splitting it would credit the reader with "east" and
        // "capital" twenty-two times over.
        assert_eq!(bases("東京"), vec!["東京"], "a name is never decomposed");
    }

    /// The other direction: a compound Sudachi's own lexicon does not hold, so
    /// no mode returns it whole and only recomposition can recover it.
    #[test]
    #[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
    fn adjacent_parts_rejoin_into_the_word_the_dictionary_lists() {
        let dict_path = std::env::var("JP_TOOLS_SUDACHI_DICT_PATH")
            .expect("JP_TOOLS_SUDACHI_DICT_PATH must be set");
        let lexicon: HashSet<String> = [
            "噦り上げる",
            "振り返る",
            "相する",
            "今年",
            "訂",
            "板",
            "東京",
            "上げる",
            "返る",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        // Readings as the master dictionary gives them, including the two that
        // tempt a wrong merge out of a kana run.
        let readings: Vec<(String, String)> = [
            ("噦り上げる", "しゃくりあげる"),
            ("振り返る", "ふりかえる"),
            ("相する", "そうする"),
            ("今年", "ことし"),
        ]
        .iter()
        .map(|(t, r)| (t.to_string(), r.to_string()))
        .collect();
        let tokenizer = SudachiTokenizer::new(Path::new(&dict_path), HashSet::from(["x".into()]))
            .unwrap()
            .with_lexicon(lexicon)
            .with_master_readings(&readings);
        let bases = |text: &str| {
            tokenizer
                .tokenize(text)
                .unwrap()
                .into_iter()
                .map(|t| t.base_form)
                .collect::<Vec<_>>()
        };

        // The case that started this: Sudachi has no しゃくりあげる, so Mode C
        // already hands back しゃくり + あげ. Rejoined on the *reading*, and
        // stored under the master's own kanji spelling.
        assert!(
            bases("しゃくりあげながら泣いた").contains(&"噦り上げる".to_string()),
            "しゃくり + あげ must rejoin"
        );
        // Rejoined on the spelling, which needs no reading index at all.
        assert!(bases("後ろを振り返った").contains(&"振り返る".to_string()));

        // The fences. そう + する *reads* like 相する, and こと + し like 今年;
        // both are listed, and both merges would be wrong. Only verb + verb
        // may match on a reading.
        assert!(
            !bases("そうすると決めた").contains(&"相する".to_string()),
            "adverb + verb must not merge on a reading alone"
        );
        assert!(!bases("ことしかできない").contains(&"今年".to_string()));
        // ていた reads as 訂 + 板 and is listed as both. Particles are not
        // content words, so the run never becomes a candidate.
        assert!(!bases("読んでいた").contains(&"訂".to_string()));
        // A name beside a word is not a compound, in either direction.
        assert_eq!(bases("東京"), vec!["東京"]);
    }

    /// A recomposed token carries the master's reading, not one assembled from
    /// the parts — the ledger keys on `(headword, reading)`, so a reading built
    /// here would be a second row for a word that already has one.
    #[test]
    #[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
    fn a_rejoined_token_carries_the_dictionarys_own_reading() {
        let dict_path = std::env::var("JP_TOOLS_SUDACHI_DICT_PATH")
            .expect("JP_TOOLS_SUDACHI_DICT_PATH must be set");
        let tokenizer = SudachiTokenizer::new(Path::new(&dict_path), HashSet::from(["x".into()]))
            .unwrap()
            .with_lexicon(HashSet::from(["噦り上げる".to_string()]))
            .with_master_readings(&[("噦り上げる".to_string(), "しゃくりあげる".to_string())]);

        let token = tokenizer
            .tokenize("しゃくりあげながら")
            .unwrap()
            .into_iter()
            .find(|t| t.base_form == "噦り上げる")
            .expect("rejoined");
        assert_eq!(token.reading, "しゃくりあげる");
        assert_eq!(token.surface, "しゃくりあげ", "the text as written");
        assert_eq!(token.pos, "動詞");
    }

    /// A reading naming two headwords is not arbitrated — it is dropped, and
    /// the parts stay apart rather than being merged into a guess.
    #[test]
    #[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
    fn an_ambiguous_reading_never_joins_anything() {
        let dict_path = std::env::var("JP_TOOLS_SUDACHI_DICT_PATH")
            .expect("JP_TOOLS_SUDACHI_DICT_PATH must be set");
        let readings: Vec<(String, String)> =
            [("持ち上げる", "もちあげる"), ("餅上げる", "もちあげる")]
                .iter()
                .map(|(t, r)| (t.to_string(), r.to_string()))
                .collect();
        let tokenizer = SudachiTokenizer::new(Path::new(&dict_path), HashSet::from(["x".into()]))
            .unwrap()
            .with_lexicon(HashSet::from(["上げる".to_string()]))
            .with_master_readings(&readings);

        let bases: Vec<String> = tokenizer
            .tokenize("もちあげる")
            .unwrap()
            .into_iter()
            .map(|t| t.base_form)
            .collect();
        assert!(
            !bases.contains(&"持ち上げる".to_string()) && !bases.contains(&"餅上げる".to_string()),
            "もちあげる names two headwords, so it names none: {bases:?}"
        );
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
