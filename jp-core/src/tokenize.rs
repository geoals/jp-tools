use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

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
    /// The two differ where Japanese spells one word several ways —
    /// いう/言う, できる/出来る, みんな/皆 — and keying on the dictionary form
    /// makes each spelling a separate word with its own counts and status.
    /// Normalization subsumes inflection too (振っ → 振る).
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
    /// Whether Sudachi calls this 非自立可能 — a word that can be the auxiliary
    /// half of a compound predicate (て**みる**, て**いた**, なければ**なら**ない).
    ///
    /// Kept because Sankoku lists those auxiliary senses as their own kana
    /// headwords, separate from the kanji verb, so the identity ladder can send
    /// them there instead of to 見る and 居る.
    pub subsidiary: bool,
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
    /// Master headwords, keyed by their reading. A reading naming several
    /// headwords keeps all of them: joining still refuses to guess, but
    /// [`SudachiTokenizer::resolve_identity`] may arbitrate by frequency.
    /// Empty disables reading-matched recomposition.
    by_reading: HashMap<String, Vec<String>>,
    /// A master headword's own reading, so a recomposed token carries the
    /// reading the ledger will key it on rather than one built from parts.
    term_reading: HashMap<String, String>,
    /// The master's `(headword, reading)` pairs — the thing an identity has to
    /// be one of. Same key shape as [`MasterWords`], which is the consumer of
    /// the identities this produces.
    pairs: HashSet<(String, String)>,
    /// BCCWJ rank per headword, to break a reading that names several. Empty
    /// refuses to arbitrate.
    frequency: HashMap<String, i64>,
    /// Readings of headwords re-tokenized standalone, for the ladder's last
    /// repair step. It fires rarely, but on words that recur forever.
    rederived: Mutex<HashMap<String, String>>,
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
            pairs: HashSet::new(),
            frequency: HashMap::new(),
            rederived: Mutex::new(HashMap::new()),
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
    /// This is also what supplies the master's `(headword, reading)` pairs, so
    /// [`resolve_identity`](Self::resolve_identity) can check an identity
    /// against the dictionary rather than assembling one from two authorities.
    ///
    /// Separate from [`with_lexicon`](Self::with_lexicon) because as a *join*
    /// signal it is the weaker one. A reading naming more than one headword is
    /// never joined on — おこす is both 起こす and 興す, and merging two tokens
    /// into a guess is worse than leaving them apart.
    pub fn with_master_readings(mut self, entries: &[(String, String)]) -> Self {
        for (term, reading) in entries {
            if reading.is_empty() {
                continue;
            }
            let reading = crate::text::kana::to_hiragana(reading);
            self.pairs.insert((term.clone(), reading.clone()));
            self.term_reading
                .entry(term.clone())
                .or_insert_with(|| reading.clone());
            let terms = self.by_reading.entry(reading).or_default();
            if !terms.contains(term) {
                terms.push(term.clone());
            }
        }
        self
    }

    /// Teach it how common each master headword is, so a reading naming several
    /// of them can still yield an identity. See the ladder's reading fallback in
    /// [`resolve_identity`](Self::resolve_identity).
    pub fn with_frequency(mut self, ranks: HashMap<String, i64>) -> Self {
        self.frequency = ranks;
        self
    }

    /// Does the master dictionary list this identity? Same rule as
    /// [`MasterWords::lists`] — a kana headword stores no reading, so it matches
    /// on the headword alone.
    fn lists(&self, headword: &str, reading: &str) -> bool {
        if crate::text::kana::is_all_kana(headword) {
            return self.lexicon.contains(headword);
        }
        self.pairs.contains(&(
            headword.to_string(),
            crate::text::kana::to_hiragana(reading),
        ))
    }

    /// The headword a reading names, arbitrated by frequency when it names
    /// several. No rank for any candidate — refuse rather than guess blind.
    fn headword_for_reading(&self, reading: &str) -> Option<&String> {
        let terms = self.by_reading.get(reading)?;
        match terms.as_slice() {
            [one] => Some(one),
            many => many
                .iter()
                .filter_map(|t| self.frequency.get(t).map(|r| (r, t)))
                .min_by_key(|(r, _)| **r)
                .map(|(_, t)| t),
        }
    }

    /// Split a compound no dictionary lists into parts that one does.
    ///
    /// Sudachi's splitting stops at its own entries: 懲罰房 is one entry with no
    /// sub-units, so every mode returns it whole and 懲罰 — a Sankoku word read
    /// sixty-one times — was credited to nothing, while 医務室 splits fine.
    ///
    /// So: longest match from the left, every part a master headword, the whole
    /// string consumed, at least two parts.
    ///
    /// **A one-character part must be kanji.** Bare kana shreds — み is a noun,
    /// so 楽しみ split into 楽し + み, and ミリア into three letters. A compound
    /// ending in kana is left whole.
    ///
    /// `None` when it cannot be built from known words, which leaves it whole
    /// and visible in the non-vocabulary tail.
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
    /// The mirror of [`decompose`](Self::decompose), and the case neither it nor
    /// the C→B→A pass can reach — both only ever *split*, so a compound
    /// Sudachi's lexicon lacks is gone before any of this logic sees it.
    /// しゃくりあげる is not a Sudachi entry, so Mode C returned しゃくり + あげ
    /// and credited しゃくる and 上げる while 噦り上げる was never met once. Over
    /// the first 14.5k lines that was 570 distinct compounds across 1,663
    /// occurrences (落ち着く, 思い出す, 振り返る…).
    ///
    /// Longest match first, left to right, at most [`MAX_COMPOUND_PARTS`] parts,
    /// joined on either signal:
    ///
    /// - **spelling** — the parts as written spell a master headword
    ///   (振り + 返る → 振り返る).
    /// - **reading** — the parts read as one (しゃくり + あげる → 噦り上げる),
    ///   needed because the text writes in kana what the dictionary spells in
    ///   kanji.
    ///
    /// **The reading signal is fenced to verb + verb with kana heads**, or
    /// そう + する merges into 相する and こと + し into 今年.
    ///
    /// Three guards apply to both: every part a content word (or ていた becomes
    /// 訂 + 板), **no part a proper noun** (a name beside a noun is a compound
    /// waiting to be invented), and three characters minimum, which keeps
    /// two-character kana homographs out.
    ///
    /// The joined token takes the master's own spelling and reading.
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
        if run.iter().any(|t| t.surface.is_empty() || t.proper_noun) {
            return None;
        }
        let (last, head) = run.split_last()?;
        let content = run.iter().all(|t| is_content_word(&t.pos));
        let surfaces: String = run.iter().map(|t| t.surface.as_str()).collect();

        let written: String = head
            .iter()
            .map(|t| t.surface.as_str())
            .chain(std::iter::once(last.base_form.as_str()))
            .collect();
        let term = if content && self.lexicon.contains(&written) {
            Some(written)
        } else if self.lexicon.contains(&surfaces) {
            // The expression join, and the one place function words are allowed
            // in: それどころか is a Sankoku headword whose parts are two
            // particles. The join is safe because what it produces must itself
            // be a listed headword — the dictionary decides wordhood, not the
            // tags of the pieces.
            Some(surfaces.clone())
        } else if self.reading_join_admitted(run, head, content) {
            let read: String = head
                .iter()
                .map(spoken_form)
                .chain(std::iter::once(crate::text::kana::to_hiragana(
                    &last.reading,
                )))
                .collect();
            // A join is a merge of two tokens; unlike an identity it may not be
            // arbitrated by frequency, so an ambiguous reading names nothing.
            match self.by_reading.get(&read).map(Vec::as_slice) {
                Some([one]) => Some(one.clone()),
                _ => None,
            }
        } else {
            None
        }?;

        if term.chars().count() < 3 {
            return None;
        }
        let reading = self
            .term_reading
            .get(&term)
            .cloned()
            .unwrap_or_else(|| last.reading.clone());
        // The joined token is an identity like any other and has to be one the
        // master lists; a join that produces something else is a bad join.
        if !self.pairs.is_empty() && !self.lists(&term, &reading) {
            return None;
        }
        Some(Joined {
            parts: run.len(),
            token: Token {
                surface: surfaces,
                reading,
                base_form: term,
                pos: last.pos.clone(),
                proper_noun: false,
                subsidiary: false,
            },
        })
    }

    /// Whether a run may be joined on its reading alone — the weak signal, and
    /// the one that invents words when it is let loose.
    ///
    /// Two admissions, both narrow:
    ///
    /// - **verb + verb, kana heads** — しゃくり + あげる → 噦り上げる.
    /// - **a kanji in the head** — 綺麗 + ごと → きれいごと → 綺麗事. The
    ///   disasters that fenced this off (そう + する → 相する, こと + し → 今年)
    ///   are all-kana runs, which this does not admit. A 接尾辞 is allowed here
    ///   and nowhere else: it is the usual second half of such a compound.
    fn reading_join_admitted(&self, run: &[Token], head: &[Token], content: bool) -> bool {
        let kana_heads = || {
            head.iter()
                .all(|t| crate::text::kana::is_all_kana(&t.surface))
        };
        if content && run.iter().all(|t| t.pos == "動詞") && kana_heads() {
            return true;
        }
        let joinable = |t: &Token| is_content_word(&t.pos) || t.pos == "接尾辞";
        run.iter().all(joinable)
            && head
                .iter()
                .any(|t| t.surface.chars().any(crate::text::kanji::is_kanji))
    }
}

/// How many tokens [`SudachiTokenizer::recompose`] will join at once. Three
/// covers 申し訳ない and もう一度; beyond that the candidates stop being
/// compounds and start being phrases, which a vocabulary ledger does not want.
const MAX_COMPOUND_PARTS: usize = 3;

/// The headwords that share a reading with another headword — the only ones
/// [`SudachiTokenizer::with_frequency`] can ever be asked about, so the only
/// ones worth fetching ranks for.
pub fn ambiguous_headwords(entries: &[(String, String)]) -> Vec<String> {
    let mut by_reading: HashMap<String, Vec<&String>> = HashMap::new();
    for (term, reading) in entries {
        if reading.is_empty() {
            continue;
        }
        let terms = by_reading
            .entry(crate::text::kana::to_hiragana(reading))
            .or_default();
        if !terms.contains(&term) {
            terms.push(term);
        }
    }
    let mut out: HashSet<String> = HashSet::new();
    for terms in by_reading.into_values().filter(|t| t.len() > 1) {
        out.extend(terms.into_iter().cloned());
    }
    out.into_iter().collect()
}

/// How a token part sounds inside a compound: as written when it is kana, by
/// its reading when it is not. Uninflected heads read the same either way, and
/// only heads reach this.
fn spoken_form(t: &Token) -> String {
    if crate::text::kana::is_all_kana(&t.surface) {
        crate::text::kana::to_hiragana(&t.surface)
    } else {
        crate::text::kana::to_hiragana(&t.reading)
    }
}

/// A joined run, and how many tokens it consumed.
struct Joined {
    token: Token,
    parts: usize,
}

impl SudachiTokenizer {
    /// The reading of the morpheme's **dictionary form**, not of its surface.
    ///
    /// `Morpheme::reading_form` is the reading of the surface: 振って gives フッ.
    /// Paired with `dictionary_form` (the lemma, 振る) that produces 振る/ふっ,
    /// a term nobody ever wrote, and splits one word across as many rows as it
    /// has inflected stems — 知る appeared as しる, しら and しっ.
    ///
    /// Sudachi knows the answer: a conjugated entry carries the word id of its
    /// dictionary form. It resolves that id for the *surface* of the dictionary
    /// form and stops, so the reading has to be asked for separately.
    ///
    /// Falls back to the surface reading when there is no dictionary form to
    /// consult, where the two are the same thing anyway.
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

    /// The `(headword, reading)` the ledger will key this morpheme on.
    ///
    /// The pair has to be one the master dictionary actually lists, **checked as
    /// a pair**. Choosing the spelling and the reading independently — a
    /// spelling validated against the headword set, a reading taken from
    /// Sudachi's lemma — is what produced (行く, いける), (一寸, ちょっと) and
    /// (私達, わたし): identities no dictionary has, so no amount of reading them
    /// ever moved the vocabulary scale.
    ///
    /// So: try candidates in order, take the first the master lists. They run
    /// from the most canonical spelling to the most surface-faithful one, so the
    /// common case (Sudachi and Sankoku agree) is settled by the first lookup and
    /// orthography folding — いう/言う as one row — is preserved.
    ///
    /// Nothing validates: keep Sudachi's own answer. The token then sits off the
    /// master scale exactly as it does today.
    fn resolve_identity<T: DictionaryAccess>(
        &self,
        m: &Morpheme<'_, T>,
        subsidiary: bool,
    ) -> (String, String) {
        let lemma_reading = self.dictionary_form_reading(m);
        let surface = m.surface().to_string();
        let sudachi = || (m.normalized_form().to_string(), lemma_reading.clone());

        // A shred, not a word — and normalisation will happily "repair" it into
        // one (んっと → うんと). It gets no candidates at all.
        if has_impossible_onset(&surface) {
            return (surface, m.reading_form().to_string());
        }
        if self.pairs.is_empty() && self.lexicon.is_empty() {
            return sudachi();
        }

        let mut candidates: Vec<(String, String)> = Vec::with_capacity(4);
        // Sankoku lists the auxiliary senses (みる, いる, なる, おく, しまう,
        // くる) as their own kana headwords, and Sudachi's dictionary form keeps
        // the surface's orthography — so this is that headword exactly. A
        // subsidiary written in kanji (見てみる's first 見る) has none of this.
        if subsidiary && crate::text::kana::is_all_kana(m.dictionary_form()) {
            candidates.push((m.dictionary_form().to_string(), lemma_reading.clone()));
        }
        candidates.push(sudachi());
        candidates.push((m.dictionary_form().to_string(), lemma_reading.clone()));
        candidates.push((surface.clone(), m.reading_form().to_string()));

        if let Some(hit) = candidates
            .iter()
            .find(|(term, reading)| self.lists(term, reading))
        {
            return hit.clone();
        }

        // The spelling is right and only the reading is wrong: Sudachi
        // normalizes a potential form to its base verb but keeps the potential's
        // reading, giving (行く, いける). Ask it what that spelling reads as on
        // its own.
        for (term, _) in &candidates {
            if !self.lexicon.contains(term) {
                continue;
            }
            if let Some(reading) = self.rederive_reading(term)
                && self.lists(term, &reading)
            {
                return (term.clone(), reading);
            }
        }

        // Only the reading is left to go on: うかがう is a word Sankoku has, but
        // only under 伺う and 窺う.
        //
        // **Hiragana surfaces only.** A reading is the weakest signal there is
        // and everything else in the language is homophonous with something:
        // katakana turned エマ into 絵馬 and トン into 頓, and Sudachi reads a
        // stray latin letter or digit aloud, so g became グラム 14,314 times and
        // 4 became 四. A word written in hiragana is the one case where the
        // reading *is* how it was written.
        if surface.chars().all(crate::text::kana::is_hiragana) {
            let spoken = crate::text::kana::to_hiragana(&surface);
            if let Some(term) = self.headword_for_reading(&spoken) {
                return (term.clone(), spoken);
            }
        }

        sudachi()
    }

    /// What a headword reads as when tokenized alone, cached forever.
    fn rederive_reading(&self, term: &str) -> Option<String> {
        if let Some(hit) = self.rederived.lock().ok()?.get(term) {
            return Some(hit.clone());
        }
        let tokenizer = StatelessTokenizer::new(&self.dict);
        let morphemes = tokenizer.tokenize(term, Mode::C, false).ok()?;
        let reading = match morphemes.iter().collect::<Vec<_>>().as_slice() {
            [m] if m.dictionary_form() == term => self.dictionary_form_reading(m),
            _ => return None,
        };
        self.rederived
            .lock()
            .ok()?
            .insert(term.to_string(), reading.clone());
        Some(reading)
    }
}

/// No Japanese word begins with っ, ん or a small kana — a token that does is a
/// shred off an out-of-vocabulary path, not a word.
///
/// Sankoku does list っ, and its sightings were all shreds; that is the point.
pub fn has_impossible_onset(surface: &str) -> bool {
    matches!(
        surface.chars().next(),
        Some(
            'っ' | 'ん'
                | 'ゃ'
                | 'ゅ'
                | 'ょ'
                | 'ぁ'
                | 'ぃ'
                | 'ぅ'
                | 'ぇ'
                | 'ぉ'
                | 'ッ'
                | 'ン'
                | 'ャ'
                | 'ュ'
                | 'ョ'
                | 'ァ'
                | 'ィ'
                | 'ゥ'
                | 'ェ'
                | 'ォ'
        )
    )
}

impl Tokenizer for SudachiTokenizer {
    fn tokenize(&self, text: &str) -> Result<Vec<Token>, TokenizeError> {
        let tokenizer = StatelessTokenizer::new(&self.dict);
        let to_token = |m: sudachi::prelude::Morpheme<'_, _>| {
            // [0] is the top-level class, [1] the subclass: 名詞,固有名詞,人名.
            let subclass = m.part_of_speech().get(1).cloned().unwrap_or_default();
            let subsidiary = subclass == "非自立可能";
            let (base_form, reading) = self.resolve_identity(&m, subsidiary);
            Token {
                surface: m.surface().to_string(),
                base_form,
                reading,
                pos: m.part_of_speech()[0].clone(),
                proper_noun: subclass == "固有名詞",
                subsidiary,
            }
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
/// (noun, pronoun, verb, adjective, adjectival noun, adverb).
///
/// 代名詞 is a top-level Sudachi tag, not a subtype of 名詞.
pub fn is_content_word(pos: &str) -> bool {
    matches!(
        pos,
        "名詞" | "代名詞" | "動詞" | "形容詞" | "形状詞" | "副詞"
    )
}

/// The master dictionary, asked the only question the affix rule needs: does it
/// list this `(headword, reading)`?
///
/// Keyed the way `knowledge::vocabulary::Term` is, and it has to be: a kana-only
/// headword stores no reading there, so asking for a pair would never match one.
/// The reading folds to hiragana for the same reason.
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
/// A content word, or anything the master dictionary lists under the reading it
/// was used with. The pair rather than the headword, because 鬼/き and 鬼/おに
/// are both Sankoku entries.
///
/// Admitting a word here is not counting it: [`COUNTS_AS_VOCAB`] takes master
/// terms only.
///
/// [`COUNTS_AS_VOCAB`]: crate::knowledge::vocabulary::COUNTS_AS_VOCAB
pub fn counts_as_word(t: &Token, master: &MasterWords) -> bool {
    if has_impossible_onset(&t.surface) {
        return false;
    }
    is_content_word(&t.pos) || master.lists(&t.base_form, &t.reading)
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
            subsidiary: false,
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
    fn a_listed_particle_counts_as_a_word() {
        // The scale is Sankoku headwords, and は is one of them. A particle is
        // admitted on the listing, not on its tag.
        let m = MasterWords::new(
            ["は".to_string()].into_iter().collect(),
            &[("は".to_string(), "は".to_string())],
        );
        assert!(counts_as_word(&affix("は", "は", "ハ", "助詞"), &m));
    }

    #[test]
    fn an_unlisted_particle_is_still_dropped() {
        assert!(!counts_as_word(&affix("は", "は", "ハ", "助詞"), &master()));
    }

    #[test]
    fn a_pronoun_counts_as_a_word() {
        // 代名詞 is not a subtype of 名詞; 彼女 fell out of the gate entirely.
        assert!(is_content_word("代名詞"));
        assert!(counts_as_word(
            &affix("彼女", "彼女", "カノジョ", "代名詞"),
            &MasterWords::new(HashSet::new(), &[])
        ));
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
