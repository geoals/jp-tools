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

pub mod trace;
use trace::{Step, Trace, Verdict};

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
    /// Whether the surface is a *stem* rather than the word itself — 続い for
    /// 続く, 許せ for 許す, なれ for 慣れる.
    ///
    /// The one thing that stops a stem being mistaken for a word. Japanese has
    /// a listed word for a great many two-kana strings, so any rule that matches
    /// a surface against the dictionary will keep finding them: 続い + て spells
    /// the conjunction 続いて, 許せ is Sankoku's imperative entry, なれ reads as
    /// 汝. None of those is what the text said.
    pub inflected: bool,
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
    /// The words already mined into Anki. A second, much smaller source for
    /// [`keeps_whole`](SudachiTokenizer::keeps_whole) — a word the reader has
    /// mined is a word — and never a substitute for `lexicon`.
    mined: HashSet<String>,
    /// Words the master dictionary lists. The wordhood authority: it decides
    /// what [`keeps_whole`](SudachiTokenizer::keeps_whole) holds together and
    /// what [`recompose`](SudachiTokenizer::recompose) may build. Empty leaves
    /// Sudachi's own analysis untouched.
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
    /// BCCWJ rank per `(headword, reading)`, to break a reading that names
    /// several. Empty refuses to arbitrate.
    frequency: HashMap<(String, String), i64>,
    /// The same ranks collapsed to the headword — its best over every reading.
    /// Only consulted when the pair itself is unranked; see
    /// [`headword_for_reading`](SudachiTokenizer::headword_for_reading).
    frequency_any_reading: HashMap<String, i64>,
    /// The reading to believe for a headword the master lists several ways,
    /// where Sudachi's choice is not to be trusted. Empty leaves it to Sudachi.
    preferred: HashMap<String, crate::knowledge::dictionaries::PreferredReading>,
    /// Master headwords the dictionary calls conjugatable lemmas. Empty means
    /// the question cannot be asked, and the structural rule stands alone.
    conjugatable: HashSet<String>,
    /// Readings of headwords re-tokenized standalone, for the ladder's last
    /// repair step. It fires rarely, but on words that recur forever.
    rederived: Mutex<HashMap<String, String>>,
}

impl SudachiTokenizer {
    pub fn new(dict_path: &Path, mined: HashSet<String>) -> Result<Self, TokenizeError> {
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
            mined,
            lexicon: HashSet::new(),
            by_reading: HashMap::new(),
            term_reading: HashMap::new(),
            pairs: HashSet::new(),
            frequency: HashMap::new(),
            frequency_any_reading: HashMap::new(),
            preferred: HashMap::new(),
            conjugatable: HashSet::new(),
            rederived: Mutex::new(HashMap::new()),
        })
    }

    /// Teach it which words the master dictionary actually lists — the set that
    /// decides whether a compound is a word.
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
    pub fn with_frequency(mut self, ranks: HashMap<(String, String), i64>) -> Self {
        for ((term, _), rank) in &ranks {
            self.frequency_any_reading
                .entry(term.clone())
                .and_modify(|best| *best = (*best).min(*rank))
                .or_insert(*rank);
        }
        self.frequency = ranks;
        self
    }

    /// Teach it which master headwords are conjugatable lemmas, so an inflected
    /// token cannot be filed under an entry that does not conjugate.
    ///
    /// Sankoku tags this on every entry (Yomitan term-bank field 3) and it is the
    /// only thing that separates 許す from 許せ: both are headwords, only one is
    /// what 許せない is a form of. Without it the tokenizer has to guess from
    /// structure alone, which cannot tell a stem that is listed from a lemma
    /// that is listed.
    pub fn with_conjugatable(mut self, terms: HashSet<String>) -> Self {
        self.conjugatable = terms;
        self
    }

    /// Whether this identity is one an *inflected* token may be filed under.
    ///
    /// A form like 許せ or 続い is a form **of** something, so the entry it lands
    /// on has to be a word that conjugates. 許せ, おいた, 汝 and 続いて are all
    /// listed, and none of them conjugates — which is exactly why matching a
    /// surface against the headword set kept finding them.
    ///
    /// Kana headwords are exempt: the auxiliaries Sankoku lists as みる, いる,
    /// なる carry no rules tag and are still what an inflected い or なら is.
    fn conjugatable_lemma(&self, term: &str) -> bool {
        self.conjugatable.is_empty()
            || self.conjugatable.contains(term)
            || crate::text::kana::is_all_kana(term)
    }

    /// Teach it which reading to believe where Sudachi's cost model is known to
    /// pick a dead one. See [`preferred_reading`](Self::preferred_reading) and
    /// `knowledge::dictionaries::preferred_readings`, which decides the map.
    pub fn with_preferred_readings(
        mut self,
        readings: HashMap<String, crate::knowledge::dictionaries::PreferredReading>,
    ) -> Self {
        self.preferred = readings;
        self
    }

    /// Correct a validated pair whose reading is one the language has moved off.
    ///
    /// This is the one place a *listed* pair is overruled, so it is deliberately
    /// hard to reach. Both 私/わたし and 私/わたくし are Sankoku pairs and every
    /// bare 私 came out わたくし, which is how 893 encounters landed on a reading
    /// almost nothing in a visual novel is.
    ///
    /// **The surface must be all kanji.** That is what makes the reading a guess
    /// worth overruling: when the text writes あたくし or わたし in kana, the
    /// reading is not Sudachi's opinion but the text's, and it stands.
    ///
    /// **And the token must be the free-standing word**, which is the only thing
    /// the popularity dictionary scored. A bound kanji is a different word that
    /// shares the spelling, and it is read the other way *because* it is bound:
    /// 数名 is メイ, 三日 is カ, 悪ガキ is ワル. Overruling those to な, にち and
    /// あく was the whole cost of this rule — 280 tokens against 私's 1,078.
    fn preferred_reading(
        &self,
        term: &str,
        reading: &str,
        surface: &str,
        pos: &[String],
    ) -> Option<String> {
        if surface.is_empty() || !surface.chars().all(crate::text::kanji::is_kanji) {
            return None;
        }
        if is_bound_morpheme(pos) {
            return None;
        }
        let pref = self.preferred.get(term)?;
        let reading = crate::text::kana::to_hiragana(reading);
        if pref.acceptable.contains(&reading) || pref.preferred == reading {
            return None;
        }
        self.lists(term, &pref.preferred)
            .then(|| pref.preferred.clone())
    }

    /// Is this compound a word in its own right, so the C→B→A pass should stop
    /// here rather than take it apart?
    ///
    /// Both dictionaries answer, and the master is the one that matters: the
    /// mined deck is a couple of thousand words, so on its own it let every
    /// compound the reader had not happened to mine be shredded into its parts.
    /// 周波数 became 周 + 波 + 数, 擦り剥く became 擦る + 剥く — three wrong
    /// identities in place of one right one, and the parts are ordinary words
    /// that nothing downstream can tell from real sightings.
    /// Asked of the normalized form as well as the dictionary form, because
    /// that is the spelling the master lists and the one an identity is keyed
    /// on: Sudachi's dictionary form for 擦り剥く is the kana すりむく, which
    /// Sankoku has no entry for.
    fn keeps_whole<T: DictionaryAccess>(&self, m: &Morpheme<'_, T>, trace: &mut Trace) -> bool {
        let forms = [m.dictionary_form(), m.normalized_form()];
        let listed = forms
            .iter()
            .find(|f| self.mined.contains(**f) || self.lexicon.contains(**f));
        if trace.is_recording() {
            let surface = m.surface().to_string();
            let why = match listed {
                Some(f) if self.lexicon.contains(*f) => format!("In master dictionary: {f}"),
                Some(f) => format!("Mined in Anki: {f}"),
                None => {
                    let mut forms: Vec<&str> = forms.to_vec();
                    forms.dedup();
                    format!(
                        "Out of vocabulary: {} not found in any dictionary",
                        forms.join(", ")
                    )
                }
            };
            let kept = listed.is_some();
            trace.push(|| Step::Gate { surface, kept, why });
        }
        listed.is_some()
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
    ///
    /// **The pair's rank, falling back to the headword's own.** The question is
    /// how common a spelling is *read this way*, and asking it of the spelling
    /// alone made いつ resolve to 一 — rank 23 as いち — over 何時, 103 times.
    /// But a pair the corpus never records must not simply lose its vote: BCCWJ
    /// ranks 先 only as さき, so keying on the pair alone dropped 先 out of the
    /// contest and handed さっき to 殺気. The fallback is the weaker signal and
    /// is used only where the sharper one is silent — where BCCWJ does rank the
    /// pair, however badly (一/いつ is 536,048th), that number stands.
    fn headword_for_reading(&self, reading: &str) -> Option<&String> {
        let terms = self.by_reading.get(reading)?;
        match terms.as_slice() {
            [one] => Some(one),
            many => many
                .iter()
                .filter_map(|t| self.rank(t, reading).map(|r| (r, t)))
                .min_by_key(|(r, _)| *r)
                .map(|(_, t)| t),
        }
    }

    fn rank(&self, term: &str, reading: &str) -> Option<i64> {
        self.frequency
            .get(&(term.to_string(), reading.to_string()))
            .or_else(|| self.frequency_any_reading.get(term))
            .copied()
    }

    /// The case neither Sudachi nor the C→B→A pass can reach — both only ever *split*, so a compound
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
    fn recompose(&self, tokens: Vec<Token>, trace: &mut Trace) -> Vec<Token> {
        if self.lexicon.is_empty() && self.by_reading.is_empty() {
            return tokens;
        }
        let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
        let mut i = 0;
        while i < tokens.len() {
            let longest = MAX_COMPOUND_PARTS.min(tokens.len() - i);
            let joined = (2..=longest).rev().find_map(|n| {
                self.join_run(
                    &tokens[i..i + n],
                    i.checked_sub(1).map(|p| &tokens[p]),
                    trace,
                )
            });
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
    fn join_run(&self, run: &[Token], before: Option<&Token>, trace: &mut Trace) -> Option<Joined> {
        let parts = || run.iter().map(|t| t.surface.clone()).collect::<Vec<_>>();
        if run.iter().any(|t| t.surface.is_empty() || t.proper_noun) {
            return no_signal(
                trace,
                parts(),
                "Blocked from merging: contains a proper noun",
            );
        }
        let (last, head) = run.split_last()?;
        let content = run.iter().all(|t| is_content_word(&t.pos));
        let surfaces: String = run.iter().map(|t| t.surface.as_str()).collect();

        let written: String = head
            .iter()
            .map(|t| t.surface.as_str())
            .chain(std::iter::once(last.base_form.as_str()))
            .collect();
        // Whether the parts *spell* the headword, as opposed to merely sounding
        // like it. The length floor below turns on this.
        let mut spelled = true;
        let mut signal = "Spelling match: parts as written form a listed headword";
        let uninflected_run = run.iter().all(|t| !t.inflected);
        let mid_conjugation = before.is_some_and(|t| conjugation_continues(t, &run[0]));
        let term = if content && self.lexicon.contains(&written) {
            Some(written)
        } else if uninflected_run && !mid_conjugation && self.lexicon.contains(&surfaces) {
            // The expression join, and the one place function words are allowed
            // in: それどころか is a Sankoku headword whose parts are two
            // particles. What it produces must itself be a listed headword — the
            // dictionary decides wordhood, not the tags of the pieces.
            //
            // **No inflected part**, and here the word class cannot stand in for
            // that rule: the entries this would produce — じゃない, として,
            // ように, しまった — are kana, and kana entries have to be exempt from
            // the conjugatable check for the auxiliaries' sake (みる, いる, なる
            // carry no rules tag either). So the structural rule holds this path:
            // a surface concatenation may not glue in a stem. Otherwise 続い + て
            // spells the conjunction 続いて and そう + な(だ) the hearsay そうな,
            // neither of which is what the sentence said — and it is what made
            // the rule look arbitrary, 開いて staying split only because Sankoku
            // happens not to list it.
            //
            // **And not starting mid-conjugation**, which the no-stem rule
            // cannot see: past た is its own dictionary form, so 音 + だっ + た +
            // そう + です left た and そう looking like two free words spelling
            // the headword たそう. They are not — た belongs to だっ, and the run
            // only looks free because the stem it hangs off sits outside it. An
            // expression never begins on the tail of the previous word's
            // inflection.
            signal = "Expression match: surfaces form a listed headword";
            Some(surfaces.clone())
        } else if self.reading_join_admitted(run, head, content) {
            spelled = false;
            signal = "Phonetic match: combined component readings form a listed headword";
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
                Some(_) => {
                    return no_signal(
                        trace,
                        parts(),
                        "Ambiguous phonetic match: reading maps to several headwords",
                    );
                }
                None => None,
            }
        } else {
            None
        };
        let Some(term) = term else {
            // The two structural guards on the expression path are refusals,
            // not misses: the run *does* spell a headword and was turned down
            // anyway, which is the case that looks like a bug and so is the one
            // that must not be folded away with the runs that spelt nothing.
            let blocked = if !uninflected_run {
                "Invalid expression: contains a bound stem rather than a full word"
            } else if mid_conjugation {
                "Invalid expression: run begins on the previous word's inflection"
            } else {
                return no_signal(
                    trace,
                    parts(),
                    "No match: parts form no listed headword by spelling or by reading",
                );
            };
            if !self.lexicon.contains(&surfaces) {
                return no_signal(
                    trace,
                    parts(),
                    "No match: parts form no listed headword by spelling or by reading",
                );
            }
            return refused(trace, parts(), surfaces, blocked.into());
        };

        // **Three characters minimum, unless the parts spell it in kanji.**
        //
        // The floor is there for kana: two kana spell so many words that a join
        // finds one by accident — こと + し is 今年, ん + だ is んだ, and the
        // reading path is worse still, since 時 + 前 sounds like 自前 and
        // 皆 + 守 like 水上. A two-kanji compound has no such ambiguity, and the
        // floor was silently costing every one Sudachi hands over in pieces:
        // 一件 came apart into 一 + 件 because Sudachi reads it as a numeral and
        // a counter, and so did 一年, 一度, 神様, 人達, 一枚, 三人, 室内 — 137
        // sightings over the corpus, none of them wrong.
        //
        // Spelled, not sounded: the reading path keeps the floor at every
        // length, because that is the one that invents 自前 out of じ + まえ.
        let unambiguous = spelled && term.chars().all(crate::text::kanji::is_kanji);
        if term.chars().count() < 3 && !unambiguous {
            return refused(
                trace,
                parts(),
                term,
                "Below length floor: under 3 characters and not written in kanji".into(),
            );
        }
        if NEVER_JOIN.contains(&term.as_str()) {
            return refused(
                trace,
                parts(),
                term,
                "Blocked from merging: master dictionary lists this string as a phrase".into(),
            );
        }
        let reading = self
            .term_reading
            .get(&term)
            .cloned()
            .unwrap_or_else(|| last.reading.clone());
        // The joined token is an identity like any other and has to be one the
        // master lists; a join that produces something else is a bad join.
        if !self.pairs.is_empty() && !self.lists(&term, &reading) {
            let reason = format!("Rejected: master dictionary does not list {term} read {reading}");
            return refused(trace, parts(), term, reason);
        }
        trace.push(|| Step::Join {
            parts: parts(),
            verdict: Verdict::Joined {
                term: term.clone(),
                reading: reading.clone(),
                signal,
            },
        });
        Some(Joined {
            parts: run.len(),
            token: Token {
                surface: surfaces,
                reading,
                base_form: term,
                pos: last.pos.clone(),
                proper_noun: false,
                subsidiary: false,
                inflected: false,
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

/// Whether `next` is the past た finishing `prev` rather than opening a word of
/// its own — the た of だっ + た.
///
/// Only the past tense, because the expressions Sankoku lists under a leading
/// auxiliary are otherwise real: ないと is 向かわ + ない + と and has to keep
/// joining. What past た heads instead are the entries that attach to a
/// *different* stem — たそう is 行きたそう, where た is たい — so a past た
/// reaching one means the run started on the tail of the previous word.
///
/// `inflected` alone will not do as the test on `prev`: it is `surface !=
/// dictionary_form`, which is also true of a normalized `……` and of every kana
/// spelling the lexicon files under another. Only a word that conjugates can
/// have its conjugation continued.
fn conjugation_continues(prev: &Token, next: &Token) -> bool {
    next.pos == "助動詞"
        && next.base_form == "た"
        && prev.inflected
        && (is_content_word(&prev.pos) || prev.pos == "助動詞")
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

/// A run that named no word at all. Free functions rather than closures over
/// the trace, so recording a refusal never has to argue with the borrow checker
/// about the recorder still being alive further down.
fn no_signal(trace: &mut Trace, parts: Vec<String>, reason: &'static str) -> Option<Joined> {
    trace.push(|| Step::Join {
        parts,
        verdict: Verdict::NoSignal { reason },
    });
    None
}

/// A run that named a word and was turned down by a later rule.
fn refused(trace: &mut Trace, parts: Vec<String>, term: String, reason: String) -> Option<Joined> {
    trace.push(|| Step::Join {
        parts,
        verdict: Verdict::Refused { term, reason },
    });
    None
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
        trace: &mut Trace,
    ) -> (String, String) {
        let (headword, reading, rule, candidates) = self.identity_ladder(m, subsidiary);
        if trace.is_recording() {
            let surface = m.surface().to_string();
            let (h, r) = (headword.clone(), reading.clone());
            trace.push(|| Step::Identity {
                surface,
                headword: h,
                reading: r,
                rule,
                candidates,
            });
        }
        (headword, reading)
    }

    /// The ladder itself, with the rung that settled it and the candidates it
    /// was choosing between — the two things an explanation needs and a caller
    /// does not.
    fn identity_ladder<T: DictionaryAccess>(
        &self,
        m: &Morpheme<'_, T>,
        subsidiary: bool,
    ) -> (String, String, &'static str, Vec<String>) {
        let lemma_reading = self.dictionary_form_reading(m);
        let surface = m.surface().to_string();
        let sudachi = || (m.normalized_form().to_string(), lemma_reading.clone());
        // Deduplicated, in ladder order: the rungs are distinct *sources* for a
        // pair — the normalized form, the lemma, the surface — and they agree
        // far more often than not. Which source won is the `rule`; four
        // identical lines under it say nothing.
        let show = |c: &[(String, String)]| {
            let mut seen = HashSet::new();
            c.iter()
                .map(|(t, r)| format!("{t} / {}", crate::text::kana::to_hiragana(r)))
                .filter(|line| seen.insert(line.clone()))
                .collect::<Vec<_>>()
        };

        // A shred, not a word — and normalisation will happily "repair" it into
        // one (んっと → うんと). It gets no candidates at all.
        if has_impossible_onset(&surface) {
            return (
                surface,
                m.reading_form().to_string(),
                "Impossible onset: fragment preserved as written",
                Vec::new(),
            );
        }
        if self.pairs.is_empty() && self.lexicon.is_empty() {
            let (t, r) = sudachi();
            return (
                t,
                r,
                "No master dictionary loaded: using Sudachi's own form and reading",
                Vec::new(),
            );
        }

        let mut candidates: Vec<(String, String)> = Vec::with_capacity(4);
        // Sankoku lists the auxiliary senses (みる, いる, なる, おく, しまう,
        // くる) as their own kana headwords, and Sudachi's dictionary form keeps
        // the surface's orthography — so this is that headword exactly. A
        // subsidiary written in kanji (見てみる's first 見る) has none of this.
        if subsidiary && crate::text::kana::is_all_kana(m.dictionary_form()) {
            candidates.push((m.dictionary_form().to_string(), lemma_reading.clone()));
        }
        // **One mora of kana never becomes a kanji word.** The reading fallback
        // already refuses this, for the reason that applies here too: Japanese
        // has a kanji for every mora, so the match is always found and never
        // evidence. UniDic normalises the か of 何もかも onto the archaic pronoun
        // 彼, which Sankoku duly lists as 彼/か — a validated pair, and not the
        // word anyone read.
        let mora_of_kana = crate::text::kana::is_all_kana(&surface)
            && is_one_mora(&crate::text::kana::to_hiragana(&surface));
        candidates.push(sudachi());
        // The normalised spelling with *its own* reading, where the lemma's is
        // not the same word's. Sudachi normalises 信じ to 信じる but reads it off
        // its dictionary form 信ずる, so the pair offered above is
        // (信じる, しんずる) — which the master does not list, and the ladder fell
        // through to 信ずる, a spelling the text never used, 104 times. Asked on
        // its own, 信じる reads しんじる and the pair lists.
        //
        // Before the dictionary form, because the normalised spelling is the one
        // the ledger keys on; a reading that came off the wrong lemma is not a
        // reason to change the spelling.
        //
        // **Only when the surface still sounds like it.** 信じ reads シンジ and
        // 信じる シンジル, so the one is a form of the other; お前 reads オマエ
        // and 御前 ゴゼン, so they are two words that happen to share a
        // normalisation, and swapping the spelling would silently rewrite the
        // sentence. まだ/未だ (マダ, イマダ) and あんた/貴方 (アンタ, アナタ)
        // are the same trap.
        if self.lexicon.contains(m.normalized_form())
            && let Some(reading) = self.rederive_reading(m.normalized_form())
            && crate::text::kana::to_hiragana(&reading)
                .starts_with(&crate::text::kana::to_hiragana(m.reading_form()))
        {
            candidates.push((m.normalized_form().to_string(), reading));
        }
        candidates.push((m.dictionary_form().to_string(), lemma_reading.clone()));
        // Only when the surface *is* the word. An inflected surface is a stem,
        // and a stem that happens to be listed is a different word: 許せ is an
        // entry of its own, and 許せない is not it.
        let uninflected = *surface == *m.dictionary_form();
        if uninflected {
            candidates.push((surface.clone(), m.reading_form().to_string()));
        }
        // A form is a form *of* something. When the surface is inflected, every
        // candidate has to be an entry that conjugates — 許せ, おいた and 汝 are
        // all listed words, and none of them is what 許せない, やっておいた or
        // なれた is a form of.
        if !uninflected {
            candidates.retain(|(term, _)| self.conjugatable_lemma(term));
        }
        // See `mora_of_kana`: one mora spells nothing, so it may keep only the
        // spelling the text used.
        if mora_of_kana {
            candidates.retain(|(term, _)| !term.chars().any(crate::text::kanji::is_kanji));
        }

        if let Some((term, reading)) = candidates
            .iter()
            .find(|(term, reading)| self.lists(term, reading))
        {
            let overruled = self.preferred_reading(term, reading, &surface, m.part_of_speech());
            let rule = if overruled.is_some() {
                "Obsolete reading replaced with the standard modern one"
            } else {
                "Exact match: master dictionary lists both spelling and reading"
            };
            let reading = overruled.unwrap_or_else(|| reading.clone());
            return (term.clone(), reading, rule, show(&candidates));
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
                return (
                    term.clone(),
                    reading,
                    "Matched by spelling: reading re-derived from the headword alone",
                    show(&candidates),
                );
            }
        }

        // Only the reading is left to go on: うかがう is a word Sankoku has, but
        // only under 伺う and 窺う.
        //
        // Asked of the **lemma**, never the surface. A surface may be a stem, and
        // a stem's sound is another word's: なれ for 慣れる reads as 汝. The
        // lemma's reading is a whole word's reading by construction, which is
        // what makes this safe to ask on an inflected token at all.
        //
        // **Words written in kana only.** A reading is the weakest signal there
        // is and everything is homophonous with something: katakana turned エマ
        // into 絵馬, and Sudachi reads a stray latin letter aloud, so g became
        // グラム 14,314 times and 4 became 四. A word written in hiragana is the
        // one case where the reading *is* how it was written.
        if m.dictionary_form()
            .chars()
            .all(crate::text::kana::is_hiragana)
        {
            let spoken = crate::text::kana::to_hiragana(&lemma_reading);
            // **Never on a single mora.** Japanese has a kanji for every one of
            // them, so this step always finds an answer and the answer is
            // always a guess: 「ぐっ」 became 具 68 times, 「ひぃ」 日 58,
            // 「ふっ」 不 41, 「ちょ、マジで」 著 22. None of those is a word
            // anyone read. A mora carries no information about which word it
            // is, and a fallback that cannot be wrong about a real word cannot
            // be right about a fragment either.
            if !is_one_mora(&spoken)
                && let Some(term) = self.headword_for_reading(&spoken)
            {
                return (
                    term.clone(),
                    spoken,
                    "Matched by reading only: hiragana token, no spelling matched",
                    show(&candidates),
                );
            }
        }

        if mora_of_kana {
            return (
                surface,
                m.reading_form().to_string(),
                "Single-mora token: too ambiguous to match, kept as written",
                show(&candidates),
            );
        }
        let (t, r) = sudachi();
        (
            t,
            r,
            "Not in master dictionary: fallback to Sudachi's own form and reading, off the master scale",
            show(&candidates),
        )
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

/// Expressions [`recompose`](SudachiTokenizer::recompose) must never build,
/// however the master spells them.
///
/// Sankoku is a learner's dictionary, so it lists a phrase like それは — the
/// intensifier of 「それはもう見事に」 — as a headword of its own. It is right
/// to. The join is what is wrong: it sees それ next to は and produces the
/// expression every time, so 「それは私の本だ」 is credited to it too. Over the
/// corpus that is 778 sightings, and それ's own count comes out at 430 when the
/// reader met it 945 times.
///
/// **A list rather than a rule, because the distinction is lexical.** 何か and
/// 何が are the same two tags, 代名詞 + 助詞; one is a word and one is a phrase.
/// ものを and ために are both 名詞 + 格助詞. Every structural rule tried here
/// took a real word with it — barring content words breaks 医務室 (名詞 +
/// 接尾辞), barring case particles breaks 本当に and ために, requiring three
/// parts keeps 中には and drops んだ. Sudachi cannot arbitrate either: it splits
/// all of them, これは and 本当に alike.
///
/// So each entry is one reviewed judgement about one string, and refusing a
/// named string cannot cost anything that is not named. Everything else the
/// join builds — ところが, まずは, 実は, 本当に, ために, すぐに, 同時に,
/// ちなみに, ところで, 医務室 — is the word the sentence used.
const NEVER_JOIN: [&str; 7] = [
    "それは", // それ + は: 「それは幸か不幸か」
    "それが", // それ + が: 「それがいつまで続くのか」
    "これは", // これ + は: 「たしかにこれは厄介ね」
    "ここに", // ここ + に: 「ここにいてほしい」
    "ものを", // もの + を: 「そぐわないものを目にし」
    "今日は", // 今日 + は — the greeting is こんにちは, and this is not it
    "たらしい", // た + らしい: the suffix of 憎たらしい, never the hearsay after
              // a past tense. Sudachi is right here and the join is not:
              // all 30 sightings are 襲われたらしい, 死んだらしい.
];

/// Remove the emphatic っ — the one written for a hard stop at the end of an
/// utterance, not to double a consonant: 早くっ, ですっ, ごめんなさいっ.
///
/// **This is the tokenizer's one edit to its input, and it happens before
/// Sudachi sees the text.** It has to: the damage is done inside the lattice,
/// so no rule over the finished tokens can undo it. An analysis that ends in a
/// 促音便 can *absorb* the っ where the whole word cannot account for it, so
/// Sudachi prefers the absorbing path and the real word loses — ごめんなさいっ
/// becomes ごめん + なさ + いっ(行く), まずいですっ becomes まず + 出る + 素っ,
/// and はいっ becomes 入る. Not fragments: different words.
///
/// A geminate is a っ with a *word* after it to double, and is never touched —
/// kana (行った, ちょっと) or kanji alike, because 突っ込む and ぶっ殺す double a
/// consonant across the okurigana just the same. Only a っ with nothing but
/// punctuation or the end of the line after it is emphatic. A word that really
/// ends in one — あっ, えっ, おっ — loses it and normalizes straight back,
/// because あ is あっ's own dictionary form.
///
/// Every character removed is a っ, so what is left is still an in-order
/// subsequence of the line and every token surface is still found in it. That
/// is what `reader::highlight::locate` needs to recover offsets against the
/// *original* text.
pub fn strip_emphatic_sokuon(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if matches!(c, 'っ' | 'ッ') {
            // Look past a run of them: にっっ is one emphatic mark, not two.
            let mut rest = chars.clone();
            let next = std::iter::from_fn(|| rest.next())
                .find(|n| !matches!(n, 'っ' | 'ッ'))
                .unwrap_or(' ');
            if !starts_a_word(next) {
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// Is there a word after this っ for it to double the consonant of? Kanji
/// counts: 突っ込む and 真っ黒 are geminates across the okurigana.
fn starts_a_word(c: char) -> bool {
    matches!(c, 'ぁ'..='ん' | 'ァ'..='ヶ' | 'ー') || crate::text::kanji::is_kanji(c)
}

/// One beat of speech: a kana, optionally followed by a small ゃゅょ.
fn is_one_mora(reading: &str) -> bool {
    let mut chars = reading.chars();
    let (Some(_), second, None) = (chars.next(), chars.next(), chars.next()) else {
        return false;
    };
    second.is_none_or(|c| matches!(c, 'ゃ' | 'ゅ' | 'ょ' | 'ャ' | 'ュ' | 'ョ'))
}

/// Is this token a piece of a compound rather than the word standing alone?
///
/// Sudachi tags the bound uses apart from the free one, which is what lets the
/// reading correction stay off them: 名 is 助数詞可能 as the counter in 数名 and
/// 一般 as the noun な, 日 is 接尾辞 in 三日, 者 is 接尾辞 in 経験者. A prefix or
/// suffix is bound by definition; a counter is bound by use, and Sudachi splits
/// the two senses into separate entries rather than tagging one token both ways.
fn is_bound_morpheme(pos: &[String]) -> bool {
    matches!(
        pos.first().map(String::as_str).unwrap_or_default(),
        "接頭辞" | "接尾辞"
    ) || matches!(
        pos.get(2).map(String::as_str).unwrap_or_default(),
        "助数詞" | "助数詞可能"
    )
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

/// Drop the leading fragment of a stammer: そ、そう / ト、ト、トラウマ.
///
/// Japanese writes a stammer by repeating the first mora with a comma after it,
/// and Sudachi analyses that fragment as whatever word it happens to spell —
/// そ is an adverb normalizing to そう, ト a particle normalizing to と. Every
/// one of those is a miscount, and the commonest of them, そ, was a sixth of
/// every そう in the corpus.
///
/// The pattern is orthographic, so the rule is too: **one kana, a comma, then a
/// word that *begins* with that same mora**, in its spelling or in its reading.
/// Kana only for the fragment — 木、木材 is not a stammer, and a kanji fragment
/// would be a real word every time.
fn drop_stutters(tokens: Vec<Token>, trace: &mut Trace) -> Vec<Token> {
    let fragment = |t: &Token| {
        let mut chars = t.surface.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            return None;
        };
        // A stammer repeats a whole mora; a small kana or っ/ん is never one.
        (crate::text::kana::is_all_kana(&t.surface) && !has_impossible_onset(&t.surface))
            .then(|| crate::text::kana::to_hiragana(&c.to_string()))
    };
    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let repeated = fragment(&tokens[i])
            .filter(|_| tokens.get(i + 1).is_some_and(|t| t.surface == "、"))
            .zip(tokens.get(i + 2))
            .is_some_and(|(mora, next)| {
                let starts_with = |s: &str| crate::text::kana::to_hiragana(s).starts_with(&mora);
                if starts_with(&next.surface) {
                    return true;
                }
                // A stammer of a word written in kanji shows the repeat only in
                // the reading — 「ち、違っ」 is ちがう and 違 carries no ち. But
                // reading off the kanji is a much weaker signal than reading it
                // off the spelling, so a *particle* is never spent on it: 「〜
                // か、彼は」 is the question particle and かれ, not a stammer, and
                // dropping the か there cost それどころか its join. The loss is
                // real — な、何 and と、父さん are stammers too — and it is the
                // cheaper one, because a wrong drop rewrites the sentence while
                // a missed one only leaves a fragment behind.
                tokens[i].pos != "助詞" && starts_with(&next.reading)
            });
        if repeated {
            let fragment = tokens[i].surface.clone();
            let into = tokens[i + 2].surface.clone();
            trace.push(|| Step::Stutter { fragment, into });
            i += 2;
            continue;
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    out
}

impl SudachiTokenizer {
    /// Tokenize, and say why — every decision the pipeline made, in order.
    ///
    /// The same code path as [`Tokenizer::tokenize`] with the recorder switched
    /// on, never a second implementation of the rules; see [`trace`].
    pub fn explain(&self, text: &str) -> Result<(Vec<Token>, Vec<Step>), TokenizeError> {
        let mut trace = Trace::recording();
        let tokens = self.run(text, &mut trace)?;
        Ok((tokens, trace.into_steps()))
    }

    fn to_token<T: DictionaryAccess>(&self, m: &Morpheme<'_, T>, trace: &mut Trace) -> Token {
        // [0] is the top-level class, [1] the subclass: 名詞,固有名詞,人名.
        let subclass = m.part_of_speech().get(1).cloned().unwrap_or_default();
        let subsidiary = subclass == "非自立可能";
        let (base_form, reading) = self.resolve_identity(m, subsidiary, trace);
        Token {
            surface: m.surface().to_string(),
            base_form,
            reading,
            pos: m.part_of_speech()[0].clone(),
            proper_noun: subclass == "固有名詞",
            subsidiary,
            inflected: *m.surface() != *m.dictionary_form(),
        }
    }

    fn run(&self, text: &str, trace: &mut Trace) -> Result<Vec<Token>, TokenizeError> {
        // The one place the input is rewritten, and the only one that can be:
        // see [`strip_emphatic_sokuon`]. Everything below analyses `text`, and
        // every surface it yields is still findable in the caller's original.
        let stripped = strip_emphatic_sokuon(text);
        if stripped != text {
            let (from, to) = (text.to_string(), stripped.clone());
            trace.push(|| Step::Rewrite { from, to });
        }
        let text = &stripped;
        let tokenizer = StatelessTokenizer::new(&self.dict);

        if self.mined.is_empty() && self.lexicon.is_empty() {
            // Nothing to ask whether a compound is a word — Mode B.
            let morphemes = tokenizer
                .tokenize(text, Mode::B, false)
                .map_err(|e| TokenizeError::Failed(e.to_string()))?;
            let plain: Vec<Token> = morphemes.iter().map(|m| self.to_token(&m, trace)).collect();
            // Still recomposed: having no wordhood gate is a reason to skip it,
            // not a reason to shred every compound the master lists.
            return Ok(self.recompose(drop_stutters(plain, trace), trace));
        }
        // Dictionary-validated splitting: C → B → A.
        // Keep tokens that are words in their own right. Split unknown
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
            if self.keeps_whole(&m, trace) {
                tokens.push(self.to_token(&m, trace));
                continue;
            }

            buf_b.clear();
            if !m.split_into(Mode::B, &mut buf_b).map_err(&err)? {
                // Mode B didn't split — try Mode A directly
                buf_a.clear();
                if m.split_into(Mode::A, &mut buf_a).map_err(&err)? {
                    self.record_split(&m, "A", &buf_a, trace);
                    for sub in buf_a.iter() {
                        tokens.push(self.to_token(&sub, trace));
                    }
                } else {
                    self.record_split(&m, "none", &buf_a, trace);
                    tokens.push(self.to_token(&m, trace));
                }
                continue;
            }

            // Mode B split — check each sub-token
            self.record_split(&m, "B", &buf_b, trace);
            for sub in buf_b.iter() {
                if self.keeps_whole(&sub, trace) {
                    tokens.push(self.to_token(&sub, trace));
                } else {
                    buf_a.clear();
                    if sub.split_into(Mode::A, &mut buf_a).map_err(&err)? {
                        self.record_split(&sub, "A", &buf_a, trace);
                        for part in buf_a.iter() {
                            tokens.push(self.to_token(&part, trace));
                        }
                    } else {
                        self.record_split(&sub, "none", &buf_a, trace);
                        tokens.push(self.to_token(&sub, trace));
                    }
                }
            }
        }

        // Recomposition goes last, over the finished stream: it has to see the
        // tokens the splitting passes actually produced, since those are what
        // shredded the compound in the first place.
        // Before recomposition: a stammer fragment is not a word, so it must
        // not be joined into one.
        Ok(self.recompose(drop_stutters(tokens, trace), trace))
    }

    fn record_split<T: DictionaryAccess>(
        &self,
        m: &Morpheme<'_, T>,
        mode: &'static str,
        parts: &sudachi::prelude::MorphemeList<T>,
        trace: &mut Trace,
    ) {
        if !trace.is_recording() {
            return;
        }
        let surface = m.surface().to_string();
        let parts: Vec<String> = parts.iter().map(|p| p.surface().to_string()).collect();
        trace.push(|| Step::Split {
            surface,
            mode,
            parts,
        });
    }
}

impl Tokenizer for SudachiTokenizer {
    fn tokenize(&self, text: &str) -> Result<Vec<Token>, TokenizeError> {
        self.run(text, &mut Trace::off())
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
    (is_content_word(&t.pos) && !is_figures(&t.surface)) || master.lists(&t.base_form, &t.reading)
}

/// A number written in figures, which Sudachi tags 名詞 and the ledger would
/// otherwise take for vocabulary.
///
/// Every distinct string of digits becomes its own headword, and Sudachi reads
/// them digit by digit, so the corpus grew 43 of them with readings like
/// 20/ニレイ, 21/ニイチ and 10/イチレイ — and 1 twice over, as イチ and ヒト.
/// The quantity in 1時間 is not a word; 時間 is.
///
/// Figures only. A numeral written in kanji is left alone, because 一 is a
/// Sankoku entry and reaches the ledger the way every other listed term does.
fn is_figures(surface: &str) -> bool {
    !surface.is_empty()
        && surface
            .chars()
            .all(|c| c.is_ascii_digit() || ('０'..='９').contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_in_figures_is_not_vocabulary() {
        let m = master();
        assert!(!counts_as_word(&affix("1", "1", "イチ", "名詞"), &m));
        assert!(!counts_as_word(&affix("２０", "20", "ニレイ", "名詞"), &m));
        // Kanji numerals are ordinary terms and reach the ledger as ones.
        assert!(counts_as_word(&affix("一", "一", "イチ", "名詞"), &m));
    }

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
            inflected: false,
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
    fn one_mora_is_a_kana_and_at_most_a_small_y() {
        assert!(is_one_mora("ぐ"));
        assert!(is_one_mora("ちょ"));
        assert!(is_one_mora("しゃ"));
        assert!(!is_one_mora("いつ"));
        assert!(!is_one_mora("うかがう"));
        assert!(!is_one_mora(""));
    }

    #[test]
    fn an_emphatic_sokuon_is_stripped_but_a_geminate_is_not() {
        assert_eq!(strip_emphatic_sokuon("ごめんなさいっ"), "ごめんなさい");
        assert_eq!(strip_emphatic_sokuon("「早くっ」"), "「早く」");
        // A run is one mark, not several.
        assert_eq!(strip_emphatic_sokuon("羽咲ちゃんっっ、"), "羽咲ちゃん、");
        assert_eq!(strip_emphatic_sokuon("ですっ……"), "です……");
        // Doubling a consonant: the っ is the word.
        assert_eq!(strip_emphatic_sokuon("行ったちょっと"), "行ったちょっと");
        assert_eq!(strip_emphatic_sokuon("待って"), "待って");
        // Across the okurigana into a kanji, which is still a geminate.
        assert_eq!(strip_emphatic_sokuon("突っ込む"), "突っ込む");
        assert_eq!(strip_emphatic_sokuon("ぶっ殺す"), "ぶっ殺す");
        assert_eq!(strip_emphatic_sokuon("真っ黒"), "真っ黒");
    }

    /// Every character removed is a っ, so what is left is still an in-order
    /// subsequence — the property `reader::highlight::locate` recovers offsets
    /// against the original line with.
    #[test]
    fn stripping_leaves_a_subsequence_of_the_original() {
        for line in [
            "「まずいですっ」",
            "「は、はいっ……」",
            "行ったっ",
            "ちょっと待ってっ",
            "力にっっ",
        ] {
            let stripped = strip_emphatic_sokuon(line);
            let mut original = line.chars();
            assert!(
                stripped.chars().all(|c| original.any(|o| o == c)),
                "{stripped:?} is not a subsequence of {line:?}"
            );
        }
    }

    fn surfaces(tokens: Vec<Token>) -> Vec<String> {
        tokens.into_iter().map(|t| t.surface).collect()
    }

    /// The fragment and its comma go; the word it stammers stays.
    #[test]
    fn a_stammer_loses_its_fragment() {
        let stream = vec![
            affix("そ", "そう", "ソ", "副詞"),
            affix("、", "、", "、", "補助記号"),
            affix("そう", "そう", "ソウ", "副詞"),
        ];
        assert_eq!(surfaces(drop_stutters(stream, &mut Trace::off())), ["そう"]);
    }

    /// ト、ト、トラウマ — a stammer may repeat, and katakana is a stammer too.
    #[test]
    fn a_repeated_stammer_loses_every_fragment() {
        let stream = vec![
            affix("ト", "と", "ト", "助詞"),
            affix("、", "、", "、", "補助記号"),
            affix("ト", "と", "ト", "助詞"),
            affix("、", "、", "、", "補助記号"),
            affix("トラウマ", "トラウマ", "トラウマ", "名詞"),
        ];
        assert_eq!(
            surfaces(drop_stutters(stream, &mut Trace::off())),
            ["トラウマ"]
        );
    }

    /// A stammer of a word written in kanji shows the repeat only in the
    /// reading: 「ち、違っ」 is ちがう, and 違 starts with no ち at all.
    #[test]
    fn a_stammer_is_caught_through_the_reading_too() {
        let stream = vec![
            affix("ち", "ちっ", "チ", "感動詞"),
            affix("、", "、", "、", "補助記号"),
            affix("違っ", "違う", "チガウ", "動詞"),
        ];
        assert_eq!(surfaces(drop_stutters(stream, &mut Trace::off())), ["違っ"]);
    }

    /// は before a name is a gasp, not a stammer — and the reading is what says
    /// so, since 羽咲 reads うさ and shares nothing with は.
    #[test]
    fn a_reading_that_does_not_repeat_is_not_a_stammer() {
        let stream = vec![
            affix("は", "は", "ハ", "助詞"),
            affix("、", "、", "、", "補助記号"),
            affix("羽咲", "羽咲", "ウサ", "名詞"),
        ];
        assert_eq!(
            surfaces(drop_stutters(stream, &mut Trace::off())),
            ["は", "、", "羽咲"]
        );
    }

    /// 「〜か、彼は」 — the question particle, and かれ. A particle is never
    /// dropped on a reading match alone; this one cost それどころか its join.
    #[test]
    fn a_particle_is_never_a_stammer_on_the_reading_alone() {
        let stream = vec![
            affix("か", "か", "カ", "助詞"),
            affix("、", "、", "、", "補助記号"),
            affix("彼", "彼", "カレ", "代名詞"),
        ];
        assert_eq!(
            surfaces(drop_stutters(stream, &mut Trace::off())),
            ["か", "、", "彼"]
        );
    }

    /// The mora has to actually repeat: そ、それ is a stammer, そ、あれ is not.
    #[test]
    fn a_comma_alone_is_not_a_stammer() {
        let stream = vec![
            affix("そ", "そう", "ソ", "副詞"),
            affix("、", "、", "、", "補助記号"),
            affix("あれ", "あれ", "アレ", "代名詞"),
        ];
        assert_eq!(
            surfaces(drop_stutters(stream, &mut Trace::off())),
            ["そ", "、", "あれ"]
        );
    }

    /// 木、木材 is two words. A one-character kanji is a word every time, which
    /// is why the rule is kana-only.
    #[test]
    fn a_kanji_fragment_is_never_a_stammer() {
        let stream = vec![
            affix("木", "木", "キ", "名詞"),
            affix("、", "、", "、", "補助記号"),
            affix("木材", "木材", "モクザイ", "名詞"),
        ];
        assert_eq!(
            surfaces(drop_stutters(stream, &mut Trace::off())),
            ["木", "、", "木材"]
        );
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

    /// A compound the master dictionary does not list stays whole.
    ///
    /// It used to be taken apart into words the master *did* list, to stop a
    /// known word inside a compound going uncredited. Measured over the corpus
    /// that traded far more than it bought: it manufactured 145 sightings of
    /// 牢屋 out of 牢屋敷 against the 13 the reader actually met, cut 味方 into
    /// "taste" + "direction", 気まずい into 気 + 不味い, and レイピア into レイ +
    /// ピア — while the two compounds it was written for stopped reaching it
    /// anyway, 懲罰房 because Sudachi calls it a place name and 医務室 because
    /// Sankoku now lists it. What it destroyed were words (味方, 裏切り,
    /// 組み合わせ); what it produced were fragments (敷き, ピア, 立て).
    ///
    /// An unlisted compound is a word the reader has not judged, and belongs in
    /// the ledger as one.
    #[test]
    #[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
    fn a_compound_the_master_does_not_list_is_left_whole() {
        let dict_path = std::env::var("JP_TOOLS_SUDACHI_DICT_PATH")
            .expect("JP_TOOLS_SUDACHI_DICT_PATH must be set");
        // Stands in for the master dictionary's headwords: every part of 白蓮華
        // is listed, and the compound itself is not.
        let lexicon: HashSet<String> = ["白蓮", "華", "東", "京", "凛", "と"]
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

        assert_eq!(bases("白蓮華"), vec!["白蓮華"]);
        assert_eq!(bases("東京"), vec!["東京"]);
        assert_eq!(bases("凛とした")[0], "凛と");
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

    /// An expression join may not start on a past た, which belongs to the verb
    /// before it. たそう is listed (行きたそう), and without this 音だったそう
    /// filed だっ's た under it.
    #[test]
    #[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
    fn a_past_tense_ta_never_opens_an_expression() {
        let dict_path = std::env::var("JP_TOOLS_SUDACHI_DICT_PATH")
            .expect("JP_TOOLS_SUDACHI_DICT_PATH must be set");
        let entries: Vec<(String, String)> = [("たそう", "たそう"), ("ないと", "ないと")]
            .iter()
            .map(|(t, r)| (t.to_string(), r.to_string()))
            .collect();
        let lexicon: HashSet<String> = entries.iter().map(|(t, _)| t.clone()).collect();
        let tokenizer = SudachiTokenizer::new(Path::new(&dict_path), HashSet::from(["x".into()]))
            .unwrap()
            .with_lexicon(lexicon)
            .with_master_readings(&entries);
        let bases = |text: &str| {
            tokenizer
                .tokenize(text)
                .unwrap()
                .into_iter()
                .map(|t| t.base_form)
                .collect::<Vec<_>>()
        };

        assert!(!bases("音だったそうです").contains(&"たそう".to_string()));
        // The fence is the past tense alone: ないと is a verb's own negative
        // plus と, and it has to keep joining.
        assert!(bases("早く向かわないと").contains(&"ないと".to_string()));
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
