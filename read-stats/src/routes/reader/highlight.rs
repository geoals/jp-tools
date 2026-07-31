//! What each word in a hooked line is worth knowing about — the spans the
//! reading view tints.
//!
//! The same Sudachi pipeline the ledger is built from ([`crate::ingest`]) runs
//! over the line as it streams, and each content word it produces is looked up
//! in `vocabulary`. What comes back is a list of *offsets*, not markup: the
//! client paints them with the CSS Custom Highlight API, so the line stays one
//! text node and Yomitan's DOM scan sees exactly what it saw before this
//! existed. That constraint is the whole reason this returns ranges.
//!
//! Two things get no span at all, and a third gets one that is not drawn:
//!
//! - **Known words**, by [`VocabRow::is_known`]'s rule — asserted known *or*
//!   mined — are sent but never painted. A line where every word you have is
//!   marked is a line you cannot read; the absence of a mark is the signal. They
//!   are sent because a span is also the region a tap judges, and a word just
//!   marked known has to stay tappable to be taken back.
//! - **Names.** Sudachi's 固有名詞 flag, same as ingest. A VN's cast would
//!   otherwise be the loudest thing on every line and learning them is not
//!   learning Japanese.
//! - **Non-words** — tokenizer noise and anything no dictionary lists. For a
//!   term the ledger already has, that is [`VocabRow::is_word`]'s lenient test;
//!   for one it does not, the master headword set. The ledger cannot be the
//!   only authority here: a word hooked ten seconds ago has no row yet, and
//!   the never-before-seen word is the one this feature exists to point at.
//!
//! The status the rest becomes is not `vocabulary.status` verbatim — the
//! ledger's `new` covers both "you have met this fifty times and never judged
//! it" and "you have never met this at all", and those are the two the reader
//! most wants told apart. [`Tier`] splits them on `encounter_count`.

use std::collections::HashSet;

use jp_core::knowledge::Knowledge;
use jp_core::knowledge::vocabulary::{self, Status, Term, VocabRow};
use jp_core::tokenize::{SudachiTokenizer, Tokenizer, is_content_word};

/// The encounter count at or below which a word is called `new` rather than
/// `seen`.
///
/// The `#vocab` tab draws the same line at zero (`unjudged_counts`), and the
/// two differ on purpose. There the question is asked of a settled ledger; here
/// it is asked of a line hooked a moment ago.
///
/// One, not zero: ingest may already have credited *this* occurrence by the
/// time the line is drawn (the sweep runs on a timer, the line arrives on a
/// 30ms poll), so a genuinely first sighting reads as either 0 or 1 depending
/// on a race nobody can observe. Erring toward `new` is the harmless side —
/// the strongest tint on a word met exactly twice costs a glance, while
/// demoting a first sighting to `seen` loses the one event worth marking.
const NEW_MAX_ENCOUNTERS: i64 = 1;

/// One word's span in the line, in UTF-16 code units from the start of the
/// text — the unit a JavaScript string is indexed in, and therefore the one a
/// `Range` offset must be. Counting `char`s here would put every highlight
/// after the line's first surrogate pair one unit to the left.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Span {
    pub start: usize,
    pub len: usize,
    /// `new`, `seen`, `unknown` or `known`. The string the mark's class is
    /// built from, so adding a status is a Rust change and a CSS rule and
    /// nothing in between.
    pub status: &'static str,
    /// The ledger key this word judges, so a tap on it can write a status
    /// without a round trip to ask what it is called. It is the *term's*
    /// spelling, not the surface under the finger: 振っ is judged as 振る.
    pub headword: String,
    pub reading: String,
}

/// What a word is to the reader, and the rule that assigns it.
///
/// `Known` is here but is not drawn: a span is also the region a tap judges,
/// and a word marked known has to stay tappable to be taken back. So the three
/// visible tiers and the invisible one are one enum, and the client decides
/// which of them gets a colour — the alternative is a word that can be judged
/// exactly once and never revisited.
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

/// The Sudachi pipeline plus the dictionary sets it needs, built once and
/// shared by every streaming reader.
///
/// Loading the dictionary costs far more than tokenizing a line does, which is
/// why [`crate::ingest`] builds one per pass over the whole history rather
/// than one per line. Here the passes are single lines arriving all evening,
/// so the load has to happen off to one side and be kept.
pub struct Highlighter {
    tokenizer: SudachiTokenizer,
    /// The master dictionary's headwords — the wordhood test for a term the
    /// ledger has no row for yet.
    lexicon: HashSet<String>,
}

impl Highlighter {
    pub fn new(tokenizer: SudachiTokenizer, lexicon: HashSet<String>) -> Highlighter {
        Highlighter { tokenizer, lexicon }
    }

    /// Every token in `text`, with its span — everything before the ledger is
    /// consulted.
    fn candidates(&self, text: &str) -> Vec<Candidate> {
        match self.tokenizer.tokenize(text) {
            Ok(tokens) => locate(text, tokens),
            Err(e) => {
                tracing::warn!(error = %e, "reader highlight tokenize failed");
                Vec::new()
            }
        }
    }

    /// Whether a term the ledger has never heard of is a word at all.
    fn in_master_lexicon(&self, term: &Term) -> bool {
        self.lexicon.contains(&term.headword)
    }
}

/// One token of the pipeline's output, placed in the line.
///
/// Not every candidate becomes a [`Span`]: a particle or a name is located all
/// the same, because `#tokenize` shows the whole token stream and "what did the
/// pipeline drop, and why" is the question that page exists to answer. The feed
/// filters them out; nothing else here does.
#[derive(Debug, Clone)]
struct Candidate {
    term: Term,
    span: Span,
    surface: String,
    pos: String,
    proper_noun: bool,
    content: bool,
}

/// Why a token carries no status — the same three exclusions this module's
/// header lists, told apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Excluded {
    /// Not a content word at all: a particle, an auxiliary, punctuation.
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
    pub reading: String,
    pub pos: String,
    /// `new`, `seen`, `unknown` or `known` — `None` for a token the feed gives
    /// no span at all.
    pub status: Option<&'static str>,
    /// Set exactly when `status` is `None`.
    pub excluded: Option<&'static str>,
    /// How often the ledger has met this term, or `None` when it has no row.
    pub encounter_count: Option<i64>,
    pub lookup_count: Option<i64>,
}

/// Pair each token with where it sits in the line.
///
/// Sudachi's tokens carry no offsets, and `decompose`/`recompose` regroup
/// surfaces without ever altering one — which is what makes a single forward
/// cursor a sound way to recover them. A surface not found ahead of the cursor
/// means that assumption broke: the token is dropped rather than guessed at,
/// because a tint on the wrong word is worse than no tint.
///
/// Free-standing and pure so the offset arithmetic — the part that silently
/// produces a plausible wrong answer — is testable without a dictionary.
fn locate(text: &str, tokens: Vec<jp_core::tokenize::Token>) -> Vec<Candidate> {
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

        let term = Term::new(t.base_form, &t.reading);
        // The tier is decided against the ledger; `Seen` is a placeholder the
        // caller overwrites, never a classification.
        let span = Span {
            start,
            len,
            status: Tier::Seen.as_str(),
            headword: term.headword.clone(),
            reading: term.reading.clone(),
        };
        out.push(Candidate {
            term,
            span,
            content: is_content_word(&t.pos),
            surface: t.surface,
            pos: t.pos,
            proper_noun: t.proper_noun,
        });
    }
    out
}

/// The process-wide [`Highlighter`], built on first use.
///
/// Not built at startup: the dictionary load is seconds of CPU, `#read` is one
/// tab out of six, and a dashboard that took that long to answer its first
/// request to serve a page nobody opened would be a poor trade. The cost lands
/// on the first line of the first session instead, where it is one lazy paint.
///
/// It is also never rebuilt, which is the honest limitation: importing a
/// dictionary changes the lexicon this was built from, and the reader picks
/// that up on the next restart. Ingest builds its own each pass, so nothing
/// *stored* is stale — only the tints, until read-stats is restarted.
pub type Shared = std::sync::Arc<tokio::sync::OnceCell<std::sync::Arc<Highlighter>>>;

/// The shared highlighter, building it if this is the first line to need it.
///
/// `None` when it could not be built — a missing Sudachi dictionary, an
/// unreadable one. The reader then streams untinted, which is what it did
/// before this existed. A failure is not memoized: the next line tries again.
pub async fn shared(state: &crate::app::AppState) -> Option<std::sync::Arc<Highlighter>> {
    let cell = state.highlighter.clone();
    let built: Result<&std::sync::Arc<Highlighter>, crate::error::AppError> = cell
        .get_or_try_init(|| async {
            let dict_path = state.sudachi_dict_path.clone();
            let vocab = crate::ingest::validation_headwords(state).await?;
            let lexicon = crate::ingest::master_lexicon(state).await?;
            let readings = crate::ingest::master_readings(state).await?;
            // Dictionary load is CPU-bound and measured in seconds; it must not
            // sit on the runtime while other readers' streams are polling.
            tokio::task::spawn_blocking(move || {
                let tokenizer = SudachiTokenizer::new(&dict_path, vocab)
                    .map_err(|e| crate::error::AppError::Upstream(format!("sudachi: {e}")))?
                    .with_lexicon(lexicon.clone())
                    .with_master_readings(&readings);
                Ok(std::sync::Arc::new(Highlighter::new(tokenizer, lexicon)))
            })
            .await
            .map_err(|e| {
                crate::error::AppError::Upstream(format!("highlighter build panicked: {e}"))
            })?
        })
        .await;
    match built {
        Ok(h) => Some(h.clone()),
        Err(e) => {
            tracing::warn!(error = %e, "reader highlighter unavailable — streaming untinted");
            None
        }
    }
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
                reading: a.reading,
            })
        })
        .collect()
}

/// Every token in `text`, classified — the feed's spans and the tokens it drops
/// in one list, in reading order.
///
/// This is what `#tokenize` renders, and it is deliberately the *same* call the
/// feed makes: a page for testing the pipeline that ran a second pipeline would
/// answer a question nobody asked. [`spans`] is this, filtered.
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
    // Judged under *one* reading is judged, full stop — `work_terms::IS_KNOWN`
    // and the triage queue both already say so, and a mark is a question, so it
    // has to obey the same rule. Sudachi gives an inflected form the reading of
    // that form (通れ → the headword 通る with the reading とおれる), so without
    // this the reading view marks a word the reader marked known, under a
    // spelling they never chose and cannot see.
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
            let mut reading = c.term.reading.clone();
            let verdict = if !c.content {
                Err(Excluded::Grammar)
            } else if c.proper_noun {
                Err(Excluded::Name)
            } else if let Some(known_reading) = known.get(&c.term.headword) {
                // A word known under another reading is known, and the span
                // points at the row that says so — a tap on it takes *that*
                // assertion back, which is the one the reader made.
                reading = known_reading.clone();
                Ok(Tier::Known)
            } else {
                match row {
                    Some(row) => tier_for(row),
                    // No row: nothing has ingested this line yet, so the ledger
                    // cannot answer and the master dictionary is asked instead.
                    None if h.in_master_lexicon(&c.term) => Ok(Tier::New),
                    None => Err(Excluded::NonWord),
                }
            };
            Analyzed {
                surface: c.surface,
                start: c.span.start,
                len: c.span.len,
                headword: c.term.headword,
                reading,
                pos: c.pos,
                status: verdict.ok().map(Tier::as_str),
                excluded: verdict.err().map(Excluded::as_str),
                encounter_count: row.map(|r| r.encounter_count),
                lookup_count: row.map(|r| r.lookup_count),
            }
        })
        .collect()
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
    use jp_core::tokenize::Token;

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
            reading: String::new(),
            pos: pos.to_string(),
            proper_noun: false,
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
        let got: Vec<Span> = marked_spans(locate(text, tokens));
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
        let got: Vec<Span> = marked_spans(locate(text, tokens));
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
        let got: Vec<Span> = marked_spans(locate(text, tokens));
        assert_eq!(
            got.iter().map(|s| (s.start, s.len)).collect::<Vec<_>>(),
            vec![(0, 1), (2, 1)]
        );
    }

    #[test]
    fn a_name_gets_no_span() {
        let mut name = token("間宮", "名詞");
        name.proper_noun = true;
        assert!(marked_spans(locate("間宮", vec![name])).is_empty());
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

    #[test]
    fn judged_unknown_outranks_its_encounter_count() {
        // A word judged unknown stays unknown however often it is met — the
        // assertion is the reader's and outranks a count.
        assert_eq!(tier_for(&row(Status::Unknown, 1)), Ok(Tier::Unknown));
        assert_eq!(tier_for(&row(Status::Unknown, 90)), Ok(Tier::Unknown));
    }
}
