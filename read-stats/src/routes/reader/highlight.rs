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
//! Three things are deliberately not highlighted:
//!
//! - **Known words**, by [`VocabRow::is_known`]'s rule — asserted known *or*
//!   mined. A line where every word you have is tinted is a line you cannot
//!   read; the absence of a mark is the signal.
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
//! most wants told apart. [`Highlight`] splits them on `encounter_count`.

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
    /// `new`, `seen` or `unknown`. Serialized as the string the CSS highlight
    /// name is built from, so adding a status is a Rust change and a CSS rule
    /// and nothing in between.
    pub status: &'static str,
}

/// The three tiers the reader sees, and the rule that assigns them.
///
/// `known` is not among them on purpose: it is the absence of a span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    /// Never judged, and never (or barely) met.
    New,
    /// Never judged, but met before. The bulk of any line.
    Seen,
    /// Judged, and judged not known.
    Unknown,
}

impl Tier {
    fn as_str(self) -> &'static str {
        match self {
            Tier::New => "new",
            Tier::Seen => "seen",
            Tier::Unknown => "unknown",
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
        Highlighter {
            tokenizer,
            lexicon,
        }
    }

    /// The candidate words in `text`, with their spans — everything before the
    /// ledger is consulted.
    fn candidates(&self, text: &str) -> Vec<(Term, Span)> {
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

/// Pair each content word with where it sits in the line.
///
/// Sudachi's tokens carry no offsets, and `decompose`/`recompose` regroup
/// surfaces without ever altering one — which is what makes a single forward
/// cursor a sound way to recover them. A surface not found ahead of the cursor
/// means that assumption broke: the token is dropped rather than guessed at,
/// because a tint on the wrong word is worse than no tint.
///
/// Free-standing and pure so the offset arithmetic — the part that silently
/// produces a plausible wrong answer — is testable without a dictionary.
fn locate(text: &str, tokens: Vec<jp_core::tokenize::Token>) -> Vec<(Term, Span)> {
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

        if !is_content_word(&t.pos) || t.proper_noun {
            continue;
        }
        let term = Term::new(t.base_form, &t.reading);
        // The tier is decided against the ledger; `Seen` is a placeholder the
        // caller overwrites, never a classification.
        let status = Tier::Seen.as_str();
        out.push((term, Span { start, len, status }));
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
    let candidates = h.candidates(text);
    if candidates.is_empty() {
        return Vec::new();
    }
    let terms: Vec<Term> = {
        let mut seen = HashSet::new();
        candidates
            .iter()
            .map(|(t, _)| t.clone())
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
    candidates
        .into_iter()
        .filter_map(|(term, span)| {
            let tier = match rows.get(&term) {
                Some(row) => tier_for(row)?,
                // No row: nothing has ingested this line yet, so the ledger
                // cannot answer and the master dictionary is asked instead.
                None if h.in_master_lexicon(&term) => Tier::New,
                None => return None,
            };
            Some(Span {
                status: tier.as_str(),
                ..span
            })
        })
        .collect()
}

/// One ledger row's tier, or `None` for a word that gets no mark at all.
fn tier_for(row: &VocabRow) -> Option<Tier> {
    if row.is_known() || row.status == Status::Blacklisted || !row.is_word() {
        return None;
    }
    match row.status {
        Status::Unknown => Some(Tier::Unknown),
        // `new` is the untriaged state, split by whether this is a first
        // meeting or the fiftieth — see NEW_MAX_ENCOUNTERS.
        _ if row.encounter_count <= NEW_MAX_ENCOUNTERS => Some(Tier::New),
        _ => Some(Tier::Seen),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jp_core::tokenize::Token;

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
        let got: Vec<Span> = locate(text, tokens).into_iter().map(|(_, s)| s).collect();
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
        let got: Vec<Span> = locate(text, tokens).into_iter().map(|(_, s)| s).collect();
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
        let tokens = vec![token("本", "名詞"), token("と", "助詞"), token("本", "名詞")];
        let got: Vec<Span> = locate(text, tokens).into_iter().map(|(_, s)| s).collect();
        assert_eq!(
            got.iter().map(|s| (s.start, s.len)).collect::<Vec<_>>(),
            vec![(0, 1), (2, 1)]
        );
    }

    #[test]
    fn a_name_gets_no_span() {
        let mut name = token("間宮", "名詞");
        name.proper_noun = true;
        assert!(locate("間宮", vec![name]).is_empty());
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
    fn known_and_mined_get_no_mark() {
        assert_eq!(tier_for(&row(Status::Known, 50)), None);
        let mut mined = row(Status::New, 50);
        mined.mined = true;
        assert_eq!(tier_for(&mined), None);
    }

    #[test]
    fn blacklisted_and_non_words_get_no_mark() {
        assert_eq!(tier_for(&row(Status::Blacklisted, 5)), None);
        let mut noise = row(Status::New, 5);
        noise.in_master = false;
        assert_eq!(tier_for(&noise), None);
    }

    #[test]
    fn untriaged_splits_on_encounter_count() {
        assert_eq!(tier_for(&row(Status::New, 1)), Some(Tier::New));
        assert_eq!(tier_for(&row(Status::New, 2)), Some(Tier::Seen));
    }

    #[test]
    fn judged_unknown_outranks_its_encounter_count() {
        // A word judged unknown stays unknown however often it is met — the
        // assertion is the reader's and outranks a count.
        assert_eq!(tier_for(&row(Status::Unknown, 1)), Some(Tier::Unknown));
        assert_eq!(tier_for(&row(Status::Unknown, 90)), Some(Tier::Unknown));
    }
}
