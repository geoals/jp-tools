//! What each word in a hooked line is worth knowing about — the spans the
//! reading view tints.
//!
//! Ingest's Sudachi pipeline runs over the line as it streams and each content
//! word is looked up in `vocabulary`. What comes back is **offsets, not
//! markup**: the client paints them, so the line stays one text node and
//! Yomitan's DOM scan sees what it always saw.
//!
//! Two things get no span, and a third gets one that is not drawn:
//!
//! - **Known words** ([`VocabRow::is_known`] — asserted known *or* mined) are
//!   sent but never painted; the absence of a mark is the signal. Sent because a
//!   span is also the region a tap judges, so a word just marked known has to
//!   stay tappable to be taken back.
//! - **Names** — the work's cast (`work_names`) first, Sudachi's 固有名詞 for
//!   anyone it does not list. A VN's cast would otherwise be the loudest thing
//!   on every line.
//! - **Non-words**: tokenizer noise, and anything no dictionary lists. The
//!   ledger answers for a term it has a row for and [`Wordhood`] for one it does
//!   not — a word hooked ten seconds ago has no row yet, and that word is what
//!   this feature exists to point at, so both must answer alike.
//!
//! [`Tier`] splits the ledger's `new` on `encounter_count`, since it covers both
//! "met fifty times, never judged" and "never met at all".

use std::collections::{HashMap, HashSet};

use crate::knowledge::Knowledge;
use crate::knowledge::vocabulary::{self, Status, Term, VocabRow};
use crate::tokenize::trace::Step;
use crate::tokenize::{MasterWords, SudachiTokenizer, Tokenizer, counts_as_word};

/// The encounter count at or below which a word is called `new` rather than
/// `seen`.
///
/// One rather than the `#vocab` tab's zero: ingest may already have credited
/// *this* occurrence by the time the line is drawn, so a first sighting reads as
/// 0 or 1 depending on a race. Erring toward `new` is the harmless side —
/// demoting a first sighting loses the one event worth marking.
const NEW_MAX_ENCOUNTERS: i64 = 1;

/// One word's span, in UTF-16 code units from the start of the text — what a
/// JavaScript `Range` offset is indexed in. Counting `char`s would put every
/// highlight after the line's first surrogate pair one unit to the left.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Span {
    pub start: usize,
    pub len: usize,
    /// `new`, `seen`, `unknown` or `known` — the mark's CSS class is built from
    /// it, so adding a status is a Rust change and a CSS rule.
    pub status: &'static str,
    /// The ledger key this word judges, so a tap on it can write a status
    /// without a round trip to ask what it is called. It is the *term's*
    /// spelling, not the surface under the finger: 振っ is judged as 振る.
    pub headword: String,
    pub reading: String,
    /// Frequency rank, `None` for a word the list does not carry. The client marks
    /// a common word it does not know more loudly than a rare one — a rare word
    /// unknown is expected, a common one is the gap worth seeing.
    pub freq_rank: Option<i64>,
    /// BCCWJ rank, `None` for a word it does not carry. Sent beside the
    /// reader-facing rank rather than instead of it: the two corpora disagree
    /// about which words are common, and a word common in either is one worth
    /// seeing. The client tests both.
    pub bccwj_rank: Option<i64>,
}

/// What a word is to the reader.
///
/// `Known` is here but not drawn: a span is also the region a tap judges, so a
/// known word stays tappable to be taken back. The client decides which tiers
/// get a colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    /// Never judged, and never (or barely) met.
    New,
    /// Never judged, but met before. The bulk of any line.
    Seen,
    /// Judged, and judged not known.
    Unknown,
    /// Asserted known, or mined. Sent, never painted.
    Known,
}

impl Tier {
    fn as_str(self) -> &'static str {
        match self {
            Tier::New => "new",
            Tier::Seen => "seen",
            Tier::Unknown => "unknown",
            Tier::Known => "known",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("knowledge database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("sudachi: {0}")]
    Sudachi(String),
}

/// The tokenizer as jp-tools configures it, plus the dictionary sets its
/// callers need beside it.
///
/// **The nine inputs are one list and every caller takes all of them.** A
/// tokenizer missing any one is a second pipeline that answers differently, and
/// the answers are compared: the reader's tint and the ledger row it is drawn
/// from have to agree about where a word ends and what it is called. Without
/// the frequency ranks alone, a kana-written word whose reading names several
/// master headwords stays kana — うかがう rather than 窺う, which no dictionary
/// lists — so the wordhood gate calls it a non-word and nothing is tinted,
/// while ingest files the same token under 窺う and counts it.
///
/// This existed three times over before it lived here: once for the reader's
/// [`Highlighter`] and twice inside read-stats' ingest.
pub struct Pipeline {
    pub tokenizer: SudachiTokenizer,
    pub lexicon: HashSet<String>,
    pub master: MasterWords,
    /// The lenient gate — every dictionary, not just the master. Here rather
    /// than only in [`Highlighter`] because ingest asks it too: a term the
    /// reader gives no span must not become a ledger row either.
    pub wordhood: Wordhood,
}

/// Fetch the nine inputs from `knowledge.db` and build the tokenizer.
///
/// The dictionary load is CPU-bound and measured in seconds, so it runs on a
/// blocking thread rather than on the runtime other requests are polling.
pub async fn pipeline(
    k: &Knowledge,
    dict_path: impl AsRef<std::path::Path>,
) -> Result<Pipeline, BuildError> {
    let pool = k.pool();
    let vocab = vocabulary::mined_vocab(pool).await?;
    let lexicon = crate::knowledge::dictionaries::master_headwords(pool).await?;
    let readings = crate::knowledge::dictionaries::master_entries(pool).await?;
    let ranks = ambiguous_ranks(pool, &readings).await?;
    let reader = reader_ranks(pool).await?;
    let preferred = preferred(pool).await?;
    let conjugatable = crate::knowledge::dictionaries::master_conjugatable(pool).await?;
    let standard = crate::knowledge::dictionaries::standard_entries(pool).await?;
    let names: HashSet<String> = crate::knowledge::work_names::all(k)
        .await?
        .into_iter()
        .collect();

    let (listed, listed_readings) = crate::knowledge::dictionaries::wordhood_entries(pool).await?;
    let wordhood = Wordhood::new(listed, listed_readings);

    let dict_path = dict_path.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || {
        let tokenizer = SudachiTokenizer::new(&dict_path, vocab)
            .map_err(|e| BuildError::Sudachi(e.to_string()))?
            .with_lexicon(lexicon.clone())
            .with_master_readings(&readings)
            .with_frequency(ranks)
            .with_reader_frequency(reader)
            .with_preferred_readings(preferred)
            .with_conjugatable(conjugatable)
            .with_standard(&standard)
            .with_names(names);
        let master = MasterWords::new(lexicon.clone(), &readings);
        Ok(Pipeline {
            tokenizer,
            lexicon,
            master,
            wordhood,
        })
    })
    .await
    .map_err(|e| BuildError::Sudachi(format!("pipeline build panicked: {e}")))?
}

/// BCCWJ ranks for the master headwords that share a reading with another, so
/// the tokenizer can name a word written in kana (うかがう → 伺う over 窺う).
///
/// **Stays on BCCWJ** where the reader-facing ranks do not: this asks which
/// spelling of one reading is the commoner one, and a list carrying kana-only
/// rows would answer it with the reading's own rank. Not being loaded is not an
/// error — ambiguous readings are then left unresolved, as they were before.
async fn ambiguous_ranks(
    pool: &sqlx::SqlitePool,
    readings: &[(String, String)],
) -> Result<HashMap<(String, String), i64>, sqlx::Error> {
    let Some(bccwj) = crate::knowledge::dictionaries::by_title(pool, "BCCWJ").await? else {
        return Ok(HashMap::new());
    };
    let terms = crate::tokenize::ambiguous_headwords(readings);
    crate::knowledge::dictionaries::frequency_ranks(pool, bccwj.id, &terms).await
}

/// The reader-facing rank per spelling, for the tokenizer's short-kana guard.
///
/// **Jiten, not BCCWJ**, for the reason `READER_FREQUENCY` exists: the question
/// is whether a spelling is one the fiction being read actually uses, and a
/// newspaper corpus ranks 船舶 eight times commoner than a novel does. Not being
/// loaded is not an error — the guard is then simply off.
async fn reader_ranks(pool: &sqlx::SqlitePool) -> Result<HashMap<String, i64>, sqlx::Error> {
    use crate::knowledge::dictionaries as d;
    let Some(jiten) = d::by_title(pool, d::READER_FREQUENCY).await? else {
        return Ok(HashMap::new());
    };
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT term, MIN(frequency) FROM dictionary_frequency
         WHERE dictionary_id = ? GROUP BY term",
    )
    .bind(jiten.id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

async fn preferred(
    pool: &sqlx::SqlitePool,
) -> Result<HashMap<String, crate::knowledge::dictionaries::PreferredReading>, sqlx::Error> {
    use crate::knowledge::dictionaries as d;
    let (Some(master), Some(jitendex), Some(bccwj)) = (
        d::master(pool).await?,
        d::by_title(pool, "Jitendex").await?,
        d::by_title(pool, "BCCWJ").await?,
    ) else {
        return Ok(HashMap::new());
    };
    d::preferred_readings(pool, master.id, jitendex.id, bccwj.id).await
}

/// Whether a term is a word at all, for a term the ledger cannot answer for.
///
/// A ledger row carries three flags and [`VocabRow::is_word`] accepts any of
/// them, so a term listed only by Jitendex is a word once ingest has seen it.
/// A term met live — which is most of what the reader is there to point at —
/// has no row yet, and asking the master alone made the same word a `non-word`
/// and cost it its span: the reader could not tap 景気づけ or ムワムワ, both of
/// which the popup would have defined. This holds the lenient set so both paths
/// ask one question.
///
/// Being a word here still says nothing about the vocabulary scale: that
/// denominator is `in_master` alone (`COUNTS_AS_VOCAB`), and the row this now
/// admits stays off it.
#[derive(Debug, Default, Clone)]
pub struct Wordhood {
    terms: HashSet<String>,
    /// Readings, for the kana half of the rule — a `Term` with no reading is a
    /// kana headword, and a dictionary giving it as some kanji headword's
    /// reading lists the word.
    readings: HashSet<String>,
}

impl Wordhood {
    pub fn new(terms: HashSet<String>, readings: HashSet<String>) -> Wordhood {
        Wordhood { terms, readings }
    }

    fn contains(&self, term: &Term) -> bool {
        self.terms.contains(&term.headword)
            || (term.reading.is_empty() && self.readings.contains(&term.headword))
    }

    /// Whether the ledger should refuse this term a row at all.
    ///
    /// The reader already gives it no span — nothing lists it, so
    /// [`contains`](Self::contains) says so — but it still becomes a row, and
    /// then the head of every triage queue: ちゅ, ぎい, ぐっ, ぎっ, ちゅぷ,
    /// くちゅ, ズチュ, グチョ are sex-scene sound effects and hook shrapnel, and
    /// nothing downstream can tell them from a word nobody has judged yet.
    ///
    /// Three morae is where the population turns. Below it, a kana string no
    /// dictionary has is almost always noise; at four and five it is the work's
    /// own vocabulary — ダイイング, トレデキム, ジンザイ, ハルウリ, ヒトカリ are
    /// what one VN is *about*, and no dictionary will ever list them.
    ///
    /// **The alphabet is asked both ways before the string is condemned.** ウチ
    /// is うち, コイツ is こいつ, ソレ is それ; a katakana spelling is a decision
    /// about how to write a word, not evidence that there is no word. That
    /// stay does not extend to **one mora**, which is the rule the identity
    /// ladder already carries: Japanese has a word for every single kana, so
    /// the match is found every time and is evidence none of them.
    ///
    /// Over the corpus: 126 terms and 338 encounters refused, and every term
    /// the fold rescues is a real word.
    pub fn is_noise(&self, term: &Term) -> bool {
        // No dictionaries loaded means the question cannot be asked, and
        // everything short and kana would answer yes. The same convention as
        // the tokenizer's empty frequency list: an absent authority refuses,
        // it does not condemn.
        if self.terms.is_empty() && self.readings.is_empty() {
            return false;
        }
        if !crate::text::kana::is_all_kana(&term.headword) || self.contains(term) {
            return false;
        }
        match crate::text::kana::morae(&term.headword) {
            1 => true,
            m if (2..=NOISE_MAX_MORAE).contains(&m) => !self.spelt_the_other_way(&term.headword),
            _ => false,
        }
    }

    /// Does a dictionary have this string in the other kana alphabet?
    fn spelt_the_other_way(&self, headword: &str) -> bool {
        let folded = crate::text::kana::to_hiragana(headword);
        folded != headword && (self.terms.contains(&folded) || self.readings.contains(&folded))
    }
}

/// How much of a word a kana string has to spell before the ledger will hold a
/// row for it that no dictionary backs — see [`Wordhood::is_noise`].
const NOISE_MAX_MORAE: usize = 3;

/// The Sudachi pipeline plus the dictionary sets it needs, built once and
/// shared by every streaming reader — the dictionary load costs far more than
/// tokenizing a line, and here the lines arrive one at a time all evening.
pub struct Highlighter {
    tokenizer: std::sync::Arc<SudachiTokenizer>,
    /// The wordhood test for a term the ledger has no row for yet — the same
    /// question [`VocabRow::is_word`] answers for a term that has one.
    wordhood: Wordhood,
    /// The same dictionary keyed by `(headword, reading)`, for the affix half of
    /// the wordhood gate. Ingest asks the identical question, which keeps a tint
    /// and a ledger row from disagreeing about 達.
    master: MasterWords,
    /// Frequency rank per `(headword, reading)`, for the master headwords. Held
    /// rather than queried per line: the reader would otherwise pay a
    /// `dictionary_frequency` lookup per word on the path that draws a line as
    /// it is being read.
    ranks: HashMap<(String, String), i64>,
    /// BCCWJ ranks for the same headwords, held for the same reason.
    bccwj_ranks: HashMap<(String, String), i64>,
}

impl Highlighter {
    /// Build one from `knowledge.db` — the [`pipeline`] plus the frequency
    /// ranks the reader needs to tell a common unknown word from a rare one.
    pub async fn build(
        k: &Knowledge,
        dict_path: impl AsRef<std::path::Path>,
    ) -> Result<Highlighter, BuildError> {
        let p = pipeline(k, dict_path).await?;
        let headwords: Vec<String> = p.lexicon.iter().cloned().collect();
        let ranks = ranks_from(
            k,
            crate::knowledge::dictionaries::READER_FREQUENCY,
            &headwords,
        )
        .await?;
        let bccwj_ranks = ranks_from(k, "BCCWJ", &headwords).await?;
        Ok(Highlighter::new(
            std::sync::Arc::new(p.tokenizer),
            p.wordhood,
            p.master,
            ranks,
            bccwj_ranks,
        ))
    }

    /// The tokenizer itself, for a caller that needs tokens rather than spans.
    ///
    /// Handed out rather than rebuilt: a second `SudachiTokenizer` is a second
    /// pipeline, and yt-mine bolds its card's target with the same analysis the
    /// popup was drawn from.
    pub fn tokenizer(&self) -> std::sync::Arc<SudachiTokenizer> {
        self.tokenizer.clone()
    }

    pub fn new(
        tokenizer: std::sync::Arc<SudachiTokenizer>,
        wordhood: Wordhood,
        master: MasterWords,
        ranks: HashMap<(String, String), i64>,
        bccwj_ranks: HashMap<(String, String), i64>,
    ) -> Highlighter {
        Highlighter {
            tokenizer,
            wordhood,
            master,
            ranks,
            bccwj_ranks,
        }
    }

    /// How common the word is. A kana headword stores no reading, so its own
    /// spelling is the reading to match; a list row that carries no reading —
    /// which is how jiten files a kana headword — answers for every reading of
    /// the spelling.
    fn rank(&self, term: &Term) -> Option<i64> {
        lookup_rank(&self.ranks, term)
    }

    /// The same word's rank in BCCWJ, which the reader tests beside the
    /// reader-facing one — see [`Span::bccwj_rank`].
    fn bccwj_rank(&self, term: &Term) -> Option<i64> {
        lookup_rank(&self.bccwj_ranks, term)
    }

    /// The ledger key a spelling from outside the tokenizer stands for — an
    /// Anki card's `VocabKanji`, the term Yomitan sends to the proxy.
    ///
    /// Asked of *this* tokenizer rather than a fresh one, for the reason the
    /// five inputs above are listed: a bare `SudachiTokenizer` is a second
    /// pipeline and answers differently. It normalizes しゃくりあげる to
    /// しゃくり上げる where this one, which knows Sankoku's spelling, gives
    /// 噦り上げる — and a key resolved by the wrong one matches no ledger row,
    /// which is the exact failure it was written to repair.
    ///
    /// A spelling that does not resolve to exactly one token is returned
    /// unchanged: a card can hold a phrase (心おきなく, 見よう見まね), and the
    /// base form of whichever fragment came back first is not that word.
    pub fn ledger_key(&self, spelling: &str) -> String {
        match self.tokenizer.tokenize(spelling) {
            Ok(tokens) => match tokens.as_slice() {
                [t] => t.base_form.clone(),
                _ => spelling.to_string(),
            },
            Err(_) => spelling.to_string(),
        }
    }

    /// Each prefix of `text` that ends on a token boundary, with its last
    /// token put back in its canonical form: しびれを切らした yields
    /// しびれを切らす.
    ///
    /// The deinflected half of the popup's match scan. A literal prefix of the
    /// line finds a compound the tokenizer split, but never an expression
    /// whose tail the sentence conjugated — and an expression is most of what
    /// a dictionary lists and the tokenizer cannot join.
    pub fn prefix_forms(&self, text: &str, max_tokens: usize) -> Vec<String> {
        let Ok(tokens) = self.tokenizer.tokenize(text) else {
            return Vec::new();
        };
        (1..=tokens.len().min(max_tokens))
            .map(|k| {
                tokens[..k - 1]
                    .iter()
                    .map(|t| t.surface.as_str())
                    .chain(std::iter::once(tokens[k - 1].base_form.as_str()))
                    .collect()
            })
            .collect()
    }

    /// Every token in `text`, with its span — everything before the ledger is
    /// consulted.
    fn candidates(&self, text: &str) -> Vec<Candidate> {
        match self.tokenizer.tokenize(text) {
            Ok(tokens) => locate(text, tokens, &self.master),
            Err(e) => {
                tracing::warn!(error = %e, "reader highlight tokenize failed");
                Vec::new()
            }
        }
    }

    /// Why the pipeline produced the tokens it did.
    ///
    /// The same tokenizer over the same text, run a second time with the
    /// recorder on. Two runs, not two pipelines: tokenizing is deterministic
    /// and pure, so the steps describe exactly the token stream
    /// [`candidates`](Self::candidates) got. A line costs microseconds and this
    /// is reached only by a hand-pasted request.
    pub fn explain(&self, text: &str) -> Vec<Step> {
        match self.tokenizer.explain(text) {
            Ok((_, steps)) => steps,
            Err(e) => {
                tracing::warn!(error = %e, "tokenizer explain failed");
                Vec::new()
            }
        }
    }

    /// Whether a term the ledger has never heard of is a word at all.
    fn is_word(&self, term: &Term) -> bool {
        self.wordhood.contains(term)
    }
}

/// One token of the pipeline's output, placed in the line.
///
/// Not every candidate becomes a [`Span`], but a particle or a name is located
/// all the same: `#tokenize` shows the whole token stream, including what the
/// pipeline dropped and why. Only the feed filters them out.
#[derive(Debug, Clone)]
struct Candidate {
    term: Term,
    span: Span,
    surface: String,
    pos: String,
    proper_noun: bool,
    content: bool,
}

/// Why a token carries no status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Excluded {
    /// Not a content word, and not a master headword either.
    Grammar,
    /// Sudachi's 固有名詞. A cast list is not vocabulary.
    Name,
    /// Judged never to surface again.
    Blacklisted,
    /// Tokenizer noise, or a term no dictionary lists.
    NonWord,
}

impl Excluded {
    fn as_str(self) -> &'static str {
        match self {
            Excluded::Grammar => "grammar",
            Excluded::Name => "name",
            Excluded::Blacklisted => "blacklisted",
            Excluded::NonWord => "non-word",
        }
    }
}

/// One token as `#tokenize` shows it: where it sits, what the ledger calls it,
/// and either the tier the feed would tint it or the reason it would not.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Analyzed {
    pub surface: String,
    pub start: usize,
    pub len: usize,
    pub headword: String,
    /// **The reading the tokenizer produced**, never another row's.
    ///
    /// The "judged under one reading is judged" substitution must not reach
    /// this field: 鬼 in 殺人鬼 is read き, and reporting it as おに because
    /// 鬼/おに is marked known would misreport the tokenizer. Where that rule
    /// fired, `judged_as` says so instead.
    pub reading: String,
    /// The reading of the row that actually carries the assertion, when the
    /// status came from a *different* row for the same headword. `None` when
    /// the status is this term's own.
    pub judged_as: Option<String>,
    pub pos: String,
    /// `new`, `seen`, `unknown` or `known` — `None` for a token the feed gives
    /// no span at all.
    pub status: Option<&'static str>,
    /// Set exactly when `status` is `None`.
    pub excluded: Option<&'static str>,
    /// How often the ledger has met this term, or `None` when it has no row.
    pub encounter_count: Option<i64>,
    pub lookup_count: Option<i64>,
    /// Frequency rank of the term, `None` when the list does not carry it.
    pub freq_rank: Option<i64>,
    /// BCCWJ rank of the term, `None` when it does not carry it.
    pub bccwj_rank: Option<i64>,
}

/// Pair each token with where it sits in the line.
///
/// Sudachi's tokens carry no offsets, but `recompose` regroups surfaces without
/// altering one, so a single forward cursor recovers them. A
/// surface not found ahead of the cursor means that assumption broke, and the
/// token is dropped rather than guessed at — a tint on the wrong word is worse
/// than no tint.
///
/// Free-standing and pure, so the offset arithmetic is testable without a
/// dictionary.
fn locate(text: &str, tokens: Vec<crate::tokenize::Token>, master: &MasterWords) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut byte_cursor = 0usize;
    let mut utf16_cursor = 0usize;
    for t in tokens {
        let Some(rel) = text[byte_cursor..].find(&t.surface) else {
            continue;
        };
        // Everything stepped over — spaces, punctuation, a dropped token —
        // still moves the UTF-16 cursor, or every span after it slides left.
        utf16_cursor += text[byte_cursor..byte_cursor + rel].encode_utf16().count();
        byte_cursor += rel + t.surface.len();
        let start = utf16_cursor;
        let len = t.surface.encode_utf16().count();
        utf16_cursor += len;

        let content = counts_as_word(&t, master);
        let term = Term::new(t.base_form, &t.reading);
        // The tier is decided against the ledger; `Seen` is a placeholder the
        // caller overwrites, never a classification.
        let span = Span {
            start,
            len,
            status: Tier::Seen.as_str(),
            headword: term.headword.clone(),
            reading: term.reading.clone(),
            freq_rank: None,
            bccwj_rank: None,
        };
        out.push(Candidate {
            term,
            span,
            content,
            surface: t.surface,
            pos: t.pos,
            proper_noun: t.proper_noun,
        });
    }
    out
}

/// The spans to tint in one line.
///
/// Best-effort by construction: a tokenizer failure or a database error yields
/// an empty list, never an error. The feed is what the reader is reading — it
/// must not stall because a word could not be classified.
pub async fn spans(k: &Knowledge, h: &Highlighter, text: &str) -> Vec<Span> {
    analyze(k, h, text)
        .await
        .into_iter()
        .filter_map(|a| {
            Some(Span {
                start: a.start,
                len: a.len,
                status: a.status?,
                headword: a.headword,
                // The span points at the row carrying the assertion, so a tap
                // takes back the judgement the reader actually made rather than
                // writing to an inflected or homographic row they never chose.
                // `analyze` keeps the two apart; only the feed folds them.
                reading: a.judged_as.unwrap_or(a.reading),
                freq_rank: a.freq_rank,
                bccwj_rank: a.bccwj_rank,
            })
        })
        .collect()
}

/// Every token in `text`, classified — the feed's spans and the tokens it drops,
/// in reading order.
///
/// What `#tokenize` renders, and deliberately the *same* call the feed makes: a
/// page for testing the pipeline must not run a second one. [`spans`] is this,
/// filtered.
pub async fn analyze(k: &Knowledge, h: &Highlighter, text: &str) -> Vec<Analyzed> {
    let candidates = h.candidates(text);
    if candidates.is_empty() {
        return Vec::new();
    }
    let terms: Vec<Term> = {
        let mut seen = HashSet::new();
        candidates
            .iter()
            .filter(|c| c.content && !c.proper_noun)
            .map(|c| c.term.clone())
            .filter(|t| seen.insert(t.clone()))
            .collect()
    };
    let rows = match vocabulary::fetch_many(k, &terms).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "reader highlight ledger lookup failed");
            return Vec::new();
        }
    };
    // Judged under one reading is judged, the same rule `work_terms::IS_KNOWN`
    // and the triage queue apply. Sudachi gives an inflected form the reading of
    // that form (通れ → 通る/とおれる), so without this the feed marks a word
    // the reader marked known, under a spelling they never chose.
    let headwords: Vec<String> = terms.iter().map(|t| t.headword.clone()).collect();
    let known = match vocabulary::known_readings(k, &headwords).await {
        Ok(known) => known,
        Err(e) => {
            tracing::warn!(error = %e, "reader highlight known-headword lookup failed");
            return Vec::new();
        }
    };
    candidates
        .into_iter()
        .map(|c| {
            let row = rows.get(&c.term);
            let mut judged_as = None;
            let verdict = if !c.content {
                Err(Excluded::Grammar)
            } else if c.proper_noun {
                Err(Excluded::Name)
            } else if let Some(known_reading) = known.get(&c.term.headword) {
                // A word known under another reading is known, and the span
                // points at the row that says so — a tap on it takes *that*
                // assertion back, which is the one the reader made.
                if known_reading != &c.term.reading {
                    judged_as = Some(known_reading.clone());
                }
                Ok(Tier::Known)
            } else {
                match row {
                    Some(row) => tier_for(row),
                    // No row: nothing has ingested this line yet, so the ledger
                    // cannot answer and the dictionaries are asked directly —
                    // the same lenient question `VocabRow::is_word` puts to a
                    // row, so a word does not lose its span for being met
                    // before ingest caught up.
                    None if h.is_word(&c.term) => Ok(Tier::New),
                    None => Err(Excluded::NonWord),
                }
            };
            let freq_rank = h.rank(&c.term);
            let bccwj_rank = h.bccwj_rank(&c.term);
            Analyzed {
                surface: c.surface,
                start: c.span.start,
                len: c.span.len,
                headword: c.term.headword,
                reading: c.term.reading,
                judged_as,
                pos: c.pos,
                status: verdict.ok().map(Tier::as_str),
                excluded: verdict.err().map(Excluded::as_str),
                encounter_count: row.map(|r| r.encounter_count),
                lookup_count: row.map(|r| r.lookup_count),
                freq_rank,
                bccwj_rank,
            }
        })
        .collect()
}

fn lookup_rank(ranks: &HashMap<(String, String), i64>, term: &Term) -> Option<i64> {
    let key = (term.headword.clone(), term.display_reading().to_string());
    ranks
        .get(&key)
        .or_else(|| ranks.get(&(term.headword.clone(), String::new())))
        .copied()
}

/// Ranks per `(headword, reading)` from one frequency dictionary, empty when it
/// is not loaded.
async fn ranks_from(
    k: &Knowledge,
    title: &str,
    headwords: &[String],
) -> Result<HashMap<(String, String), i64>, sqlx::Error> {
    use crate::knowledge::dictionaries as d;
    match d::by_title(k.pool(), title).await? {
        Some(dict) => d::frequency_ranks(k.pool(), dict.id, headwords).await,
        None => Ok(HashMap::new()),
    }
}

/// One ledger row's tier, or why the word gets no mark at all.
fn tier_for(row: &VocabRow) -> Result<Tier, Excluded> {
    // Blacklisted and non-words get no span at all, which is also what makes a
    // tap on them do nothing: there is nothing under the finger to judge.
    if row.status == Status::Blacklisted {
        return Err(Excluded::Blacklisted);
    }
    if !row.is_word() {
        return Err(Excluded::NonWord);
    }
    if row.is_known() {
        return Ok(Tier::Known);
    }
    match row.status {
        Status::Unknown => Ok(Tier::Unknown),
        // `new` is the untriaged state, split by whether this is a first
        // meeting or the fiftieth — see NEW_MAX_ENCOUNTERS.
        _ if row.encounter_count <= NEW_MAX_ENCOUNTERS => Ok(Tier::New),
        _ => Ok(Tier::Seen),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenize::Token;

    /// A master dictionary that lists nothing, for the offset tests: what
    /// `locate` does with an affix is [`counts_as_word`]'s business and is
    /// tested there.
    fn no_master() -> MasterWords {
        MasterWords::new(HashSet::new(), &[])
    }

    /// The spans the feed would draw from a located line — what `locate`
    /// returned before it started carrying the dropped tokens too.
    fn marked_spans(candidates: Vec<Candidate>) -> Vec<Span> {
        candidates
            .into_iter()
            .filter(|c| c.content && !c.proper_noun)
            .map(|c| c.span)
            .collect()
    }

    fn token(surface: &str, pos: &str) -> Token {
        Token {
            surface: surface.to_string(),
            base_form: surface.to_string(),
            dictionary_form: surface.to_string(),
            reading: String::new(),
            pos: pos.to_string(),
            proper_noun: false,
            subsidiary: false,
            counter: false,
            derived_class: None,
            inflected: false,
        }
    }

    #[test]
    fn spans_are_utf16_offsets_into_the_line() {
        let text = "彼女は本を読む";
        let tokens = vec![
            token("彼女", "名詞"),
            token("は", "助詞"),
            token("本", "名詞"),
            token("を", "助詞"),
            token("読む", "動詞"),
        ];
        let got: Vec<Span> = marked_spans(locate(text, tokens, &no_master()));
        assert_eq!(
            got.iter().map(|s| (s.start, s.len)).collect::<Vec<_>>(),
            vec![(0, 2), (3, 1), (5, 2)],
            "particles are skipped but must still advance the cursor"
        );
    }

    #[test]
    fn a_surrogate_pair_counts_as_two_units() {
        // 𠮟 is outside the BMP: one char, two UTF-16 units. Counting chars
        // here would put every later span one unit to the left of its word.
        let text = "𠮟る本";
        let tokens = vec![token("𠮟る", "動詞"), token("本", "名詞")];
        let got: Vec<Span> = marked_spans(locate(text, tokens, &no_master()));
        assert_eq!(
            got.iter().map(|s| (s.start, s.len)).collect::<Vec<_>>(),
            vec![(0, 3), (3, 1)]
        );
    }

    #[test]
    fn a_repeated_surface_lands_on_each_occurrence() {
        // The cursor only ever moves forward, so the second 本 must not be
        // matched back at the first one's offset.
        let text = "本と本";
        let tokens = vec![
            token("本", "名詞"),
            token("と", "助詞"),
            token("本", "名詞"),
        ];
        let got: Vec<Span> = marked_spans(locate(text, tokens, &no_master()));
        assert_eq!(
            got.iter().map(|s| (s.start, s.len)).collect::<Vec<_>>(),
            vec![(0, 1), (2, 1)]
        );
    }

    /// The tokenizer strips the emphatic っ before Sudachi sees the line
    /// (`crate::tokenize::strip_emphatic_sokuon`), so the surfaces it returns
    /// are missing characters the line still has. Offsets are recovered against
    /// the *original*, and the stripped っ has to be stepped over like any
    /// other unclaimed character or every later span slides left.
    #[test]
    fn a_stripped_sokuon_still_advances_the_cursor() {
        let text = "「早くっ、本を」";
        let tokens = vec![token("早く", "形容詞"), token("本", "名詞")];
        let got: Vec<Span> = marked_spans(locate(text, tokens, &no_master()));
        assert_eq!(
            got.iter().map(|s| (s.start, s.len)).collect::<Vec<_>>(),
            vec![(1, 2), (5, 1)],
            "本 sits at 5: 「早くっ、 is four units before it"
        );
    }

    #[test]
    fn a_name_gets_no_span() {
        let mut name = token("間宮", "名詞");
        name.proper_noun = true;
        assert!(marked_spans(locate("間宮", vec![name], &no_master())).is_empty());
    }

    fn row(status: Status, encounters: i64) -> VocabRow {
        VocabRow {
            term: Term::new("懲罰", "ちょうばつ"),
            pos: None,
            status,
            status_ts: None,
            mined: false,
            encounter_count: encounters,
            lookup_count: 0,
            first_seen: Some(0.0),
            last_seen: Some(0.0),
            in_master: true,
            in_name: false,
            in_reference: false,
            freq_rank: None,
        }
    }

    #[test]
    fn known_and_mined_are_sent_as_known() {
        // Sent, so a tap can take them back; the client is what declines to
        // paint them.
        assert_eq!(tier_for(&row(Status::Known, 50)), Ok(Tier::Known));
        let mut mined = row(Status::New, 50);
        mined.mined = true;
        assert_eq!(tier_for(&mined), Ok(Tier::Known));
    }

    #[test]
    fn blacklisted_and_non_words_get_no_mark() {
        assert_eq!(
            tier_for(&row(Status::Blacklisted, 5)),
            Err(Excluded::Blacklisted)
        );
        let mut noise = row(Status::New, 5);
        noise.in_master = false;
        assert_eq!(tier_for(&noise), Err(Excluded::NonWord));
    }

    #[test]
    fn untriaged_splits_on_encounter_count() {
        assert_eq!(tier_for(&row(Status::New, 1)), Ok(Tier::New));
        assert_eq!(tier_for(&row(Status::New, 2)), Ok(Tier::Seen));
    }

    fn wordhood() -> Wordhood {
        Wordhood::new(
            ["景気づけ".to_string()].into_iter().collect(),
            ["むわむわ".to_string()].into_iter().collect(),
        )
    }

    #[test]
    fn a_term_only_a_reference_dictionary_lists_is_a_word() {
        // The lenient gate, matching what a ledger row would say. Sankoku has
        // only 景気付け, so the master alone answered `non-word` and the reader
        // got no span to tap.
        assert!(wordhood().contains(&Term::new("景気づけ", "けいきづけ")));
        assert!(!wordhood().contains(&Term::new("景気づける", "けいきづける")));
    }

    #[test]
    fn a_kana_headword_matches_a_dictionary_reading() {
        // `Term` stores no reading for a kana headword, so the headword itself
        // is what a dictionary's reading column has to be matched against —
        // the same kana half `refresh_dictionary_flags` gives a row.
        assert!(wordhood().contains(&Term::new("むわむわ", "むわむわ")));
        // A kanji headword gets no such fallback: its own spelling must be
        // listed, or every homophone would be a word.
        assert!(!wordhood().contains(&Term::new("無和", "むわむわ")));
    }

    /// The sex-scene sound effects and hook shrapnel that head every triage
    /// queue: ぎい, ぐっ, ちゅぷ, ズチュ. 126 terms and 338 encounters of it.
    #[test]
    fn a_short_kana_string_no_dictionary_has_is_not_a_word() {
        let w = wordhood();
        assert!(w.is_noise(&Term::new("ぎい", "ぎい")));
        assert!(w.is_noise(&Term::new("ちゅぷ", "ちゅぷ")));
        assert!(w.is_noise(&Term::new("ズチュ", "ズチュ")));
        // A dictionary having it settles the question, whatever its length.
        assert!(!w.is_noise(&Term::new("むわむわ", "むわむわ")));
        assert!(!w.is_noise(&Term::new("景気づけ", "けいきづけ")));
    }

    /// Four morae is where the population turns: ダイイング, トレデキム,
    /// ジンザイ, ハルウリ, ヒトカリ are what one VN is *about*, and no
    /// dictionary will ever list them.
    #[test]
    fn a_longer_coinage_is_left_alone() {
        assert!(!wordhood().is_noise(&Term::new("ダイイング", "ダイイング")));
        assert!(!wordhood().is_noise(&Term::new("ヒトカリ", "ヒトカリ")));
    }

    /// A katakana spelling is a decision about how to write a word, not
    /// evidence that there is no word — so the alphabet is asked both ways.
    #[test]
    fn the_other_alphabet_is_asked_before_the_string_is_condemned() {
        let w = Wordhood::new(
            ["うち".to_string()].into_iter().collect(),
            ["こいつ".to_string()].into_iter().collect(),
        );
        assert!(!w.is_noise(&Term::new("ウチ", "ウチ")));
        assert!(!w.is_noise(&Term::new("コイツ", "コイツ")));
        assert!(w.is_noise(&Term::new("ズチュ", "ズチュ")));
    }

    /// **Except at one mora**, which is the rule the identity ladder already
    /// carries: Japanese has a word for every single kana, so 血/ち rescues the
    /// チ of a hook shred every time and is evidence none of them.
    #[test]
    fn one_mora_is_never_rescued_by_the_fold() {
        let w = Wordhood::new(HashSet::new(), ["ち".to_string()].into_iter().collect());
        assert!(w.is_noise(&Term::new("チ", "チ")));
        // Listed as itself, it is a word — が and は reach the ledger this way.
        let listed = Wordhood::new(["を".to_string()].into_iter().collect(), HashSet::new());
        assert!(!listed.is_noise(&Term::new("を", "を")));
    }

    /// No dictionaries loaded means the question cannot be asked.
    #[test]
    fn an_empty_wordhood_condemns_nothing() {
        assert!(!Wordhood::default().is_noise(&Term::new("ぎい", "ぎい")));
    }

    #[test]
    fn judged_unknown_outranks_its_encounter_count() {
        // A word judged unknown stays unknown however often it is met — the
        // assertion is the reader's and outranks a count.
        assert_eq!(tier_for(&row(Status::Unknown, 1)), Ok(Tier::Unknown));
        assert_eq!(tier_for(&row(Status::Unknown, 90)), Ok(Tier::Unknown));
    }
}
