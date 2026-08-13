//! The canonical two-axis tag rubric (FAMILIARITY + FLAVOR) shared by every LLM
//! call that emits it: the CompactDef gloss ([`crate::compactdef`]) and
//! read-stats' explain button. Both used to carry their own paraphrase of these
//! definitions and had already drifted apart; this is the single source of truth
//! so a wording change lands everywhere at once.
//!
//! FAMILIARITY is optional and usually absent. It claims something about the
//! whole native adult population, which is not knowable for most words, and a
//! forced five-tier version of it put COMMON on 74% of the collection — two
//! words checked against a native speaker came back wrong in the same
//! direction. Omission is the honest answer and the expected one.
//!
//! It is rated on the spelling the reader actually met, which is why both
//! callers send a surface form and never a dictionary headword: 饐える is a rare
//! kanji, すえた臭い is a phrase people say, and the card is about the second.

/// The FAMILIARITY axis — omitted unless the claim is safe to bet on.
pub const FAMILIARITY_RUBRIC: &str = "\
FAMILIARITY (optional — omit unless very confident) — recognition across the \
whole native adult population, including adults who read no books.\n\
Emit one only if you would bet money on it:\n\
- CORE — every native from childhood: children's TV, everyday conversation.\n\
- COMMON — every native adult knows it. Test: would an adult who reads no books \
meet this on TV, at work, on a sign, or in conversation? If it is only ever met \
in print, it is not COMMON.\n\
- RARE — you are confident a large share of natives would not recognize it.";

/// The FLAVOR axis — one baseline formality plus up to two independent marks.
pub const FLAVOR_RUBRIC: &str = "\
FLAVOR (1-3) — if you SAY it in the wrong room, how do you sound. Emit exactly \
one baseline formality, then add marks only when they carry an independent, \
equally-important warning:\n\
- baseline: SLANG / PLAIN (safe anywhere — always shown) / FORMAL (stiff if \
casual; fine in formal speech or writing).\n\
- marks: LITERARY (writing-only; theatrical if spoken), TECHNICAL, RELIGIOUS, \
HONORIFIC, HUMBLE, DIALECT, ARCHAIC, VULGAR, DEROGATORY, CHILDISH.\n\
Tag the IN-SENTENCE sense; other senses don't count (joking 成仏 = PLAIN, not \
RELIGIOUS). A word can be marked in origin but plain in use — tag current usage, \
not etymology.";

/// The FAMILIARITY tiers, most familiar first.
///
/// Three, not five. The middle of a five-tier scale is where the population
/// claim cannot be made honestly, and UNCOMMON was absorbing it — 483 cards
/// carried it. Omitting the tier says that better than a tier for it does.
pub const FAMILIARITY: [&str; 3] = ["CORE", "COMMON", "RARE"];

/// The baseline formalities. Exactly one is required.
pub const BASELINES: [&str; 3] = ["SLANG", "PLAIN", "FORMAL"];

/// The independent marks, added to a baseline. Order in the rendered line
/// follows this array, not the order the model happened to emit them in.
///
/// LITERARY is a mark and not a baseline because it answers a different
/// question: the baseline is how stiff a word is, LITERARY is which medium it
/// belongs to, and a word can be both. Forcing the choice produced 26 cards
/// that emitted FORMAL and LITERARY together — 怨恨, 傀儡, 羅列 are stiff *and*
/// writing-only, while 束の間 is writing-heavy at a plain register and 誤謬 is
/// stiff but perfectly sayable.
pub const MARKS: [&str; 10] = [
    "LITERARY",
    "TECHNICAL",
    "RELIGIOUS",
    "HONORIFIC",
    "HUMBLE",
    "DIALECT",
    "ARCHAIC",
    "VULGAR",
    "DEROGATORY",
    "CHILDISH",
];

/// The optional trailing parenthetical.
pub const STRUCTURAL: [&str; 6] = [
    "idiom",
    "mimetic",
    "fixed phrase",
    "proverb",
    "name",
    "four-char idiom",
];

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TagLineError {
    #[error("no baseline formality (one of SLANG/PLAIN/FORMAL)")]
    NoBaseline,
    #[error("more than one baseline formality: {0}")]
    TwoBaselines(String),
    #[error("unknown tag {0:?}")]
    UnknownTag(String),
    #[error("unknown structural note {0:?}")]
    UnknownStructural(String),
    #[error("empty tag line")]
    Empty,
}

/// A parsed CompactDef tag line: the two axes plus the optional structural note.
///
/// Parsing is the only way the tag line reaches a card, so the field is always
/// in one shape and can be split back apart by a script. It is deliberately
/// lenient about the *separator* and strict about the *content*: the model
/// drops the `·` between the axes often enough to matter (`COMMON FORMAL ·
/// TECHNICAL`), and that is a rendering slip with no ambiguity in it, while a
/// missing baseline or an invented tag is a judgement that was never made.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct TagLine {
    /// Absent when no familiarity claim is being made. A tier asserts something
    /// about the whole native-speaker population, which is not a judgement that
    /// can be made about every word; silence is a legitimate answer and is
    /// preferable to a tier picked to fill the slot.
    pub familiarity: Option<&'static str>,
    pub baseline: &'static str,
    pub marks: Vec<&'static str>,
    pub structural: Option<&'static str>,
}

impl TagLine {
    /// Parse a tag line, repairing separators and rejecting anything else.
    pub fn parse(line: &str) -> Result<Self, TagLineError> {
        let line = line.trim();
        if line.is_empty() {
            return Err(TagLineError::Empty);
        }

        let (tags, structural) = match line.strip_suffix(')').and_then(|l| l.rsplit_once('(')) {
            Some((tags, note)) => {
                let note = note.trim();
                let known = STRUCTURAL
                    .iter()
                    .find(|s| s.eq_ignore_ascii_case(note))
                    .ok_or_else(|| TagLineError::UnknownStructural(note.to_string()))?;
                (tags, Some(*known))
            }
            None => (line, None),
        };

        // Both `·` and whitespace separate tokens: the axis separator is what
        // goes missing, and no tag contains a space.
        let mut tokens = tags
            .split(|c: char| c == '·' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .peekable();

        // A familiarity tier, when made, leads the line; a line that opens on a
        // flavor tag is simply not making the claim.
        let familiarity = FAMILIARITY
            .iter()
            .find(|f| tokens.peek().is_some_and(|t| f.eq_ignore_ascii_case(t)))
            .copied();
        if familiarity.is_some() {
            tokens.next();
        }

        let mut baseline: Option<&'static str> = None;
        let mut marks = Vec::new();
        for token in tokens {
            if let Some(b) = BASELINES.iter().find(|b| b.eq_ignore_ascii_case(token)) {
                if let Some(had) = baseline {
                    return Err(TagLineError::TwoBaselines(format!("{had} and {b}")));
                }
                baseline = Some(b);
            } else if let Some(m) = MARKS.iter().find(|m| m.eq_ignore_ascii_case(token)) {
                if !marks.contains(m) {
                    marks.push(*m);
                }
            } else {
                return Err(TagLineError::UnknownTag(token.to_string()));
            }
        }

        marks.sort_by_key(|m| MARKS.iter().position(|k| k == m));
        Ok(Self {
            familiarity,
            baseline: baseline.ok_or(TagLineError::NoBaseline)?,
            marks,
            structural,
        })
    }
}

impl std::fmt::Display for TagLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(familiarity) = self.familiarity {
            write!(f, "{familiarity} · ")?;
        }
        write!(f, "{}", self.baseline)?;
        for mark in &self.marks {
            write!(f, " · {mark}")?;
        }
        match self.structural {
            Some(s) => write!(f, " ({s})"),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(line: &str) -> String {
        TagLine::parse(line).unwrap().to_string()
    }

    #[test]
    fn canonical_lines_survive_unchanged() {
        for line in [
            "COMMON · PLAIN",
            "RARE · FORMAL · HONORIFIC",
            "CORE · PLAIN (mimetic)",
            "RARE · FORMAL · LITERARY",
            "PLAIN · TECHNICAL",
            "COMMON · FORMAL (four-char idiom)",
        ] {
            assert_eq!(round_trip(line), line);
        }
    }

    /// The bug this parser exists for: the axis separator goes missing whenever
    /// a mark follows the baseline. Both shapes were live on real cards.
    #[test]
    fn a_missing_axis_separator_is_repaired() {
        assert_eq!(
            round_trip("COMMON FORMAL · TECHNICAL"),
            "COMMON · FORMAL · TECHNICAL"
        );
        assert_eq!(
            round_trip("RARE PLAIN · DIALECT"),
            "RARE · PLAIN · DIALECT"
        );
        assert_eq!(round_trip("COMMON PLAIN TECHNICAL"), "COMMON · PLAIN · TECHNICAL");
    }

    #[test]
    fn marks_render_in_rubric_order() {
        assert_eq!(
            round_trip("COMMON · TECHNICAL · PLAIN · ARCHAIC"),
            "COMMON · PLAIN · TECHNICAL · ARCHAIC"
        );
    }

    /// A missing baseline is not repairable — PLAIN is not a safe default, it is
    /// the claim "safe anywhere", which is exactly what a TECHNICAL-only line
    /// failed to decide.
    /// The 26-card case: stiff *and* writing-only is one word's two properties,
    /// not a choice between them.
    #[test]
    fn formal_and_literary_coexist() {
        assert_eq!(
            round_trip("RARE · LITERARY · FORMAL"),
            "RARE · FORMAL · LITERARY"
        );
    }

    /// A tier is a claim about the whole population. Declining to make it is an
    /// answer, and the line still parses.
    #[test]
    fn familiarity_is_optional() {
        let parsed = TagLine::parse("FORMAL · TECHNICAL").unwrap();
        assert_eq!(parsed.familiarity, None);
        assert_eq!(parsed.to_string(), "FORMAL · TECHNICAL");
        assert_eq!(
            TagLine::parse("COMMON · PLAIN").unwrap().familiarity,
            Some("COMMON")
        );
    }

    #[test]
    fn a_missing_baseline_is_rejected() {
        assert_eq!(
            TagLine::parse("RARE · TECHNICAL"),
            Err(TagLineError::NoBaseline)
        );
        assert_eq!(TagLine::parse("COMMON"), Err(TagLineError::NoBaseline));
        assert_eq!(TagLine::parse("TECHNICAL"), Err(TagLineError::NoBaseline));
    }

    /// The retired tiers are not silently accepted: a card still carrying one
    /// has not been re-judged under the abstaining rubric.
    #[test]
    fn retired_tiers_are_rejected() {
        for line in ["UNCOMMON · PLAIN", "OBSCURE · FORMAL"] {
            assert!(matches!(
                TagLine::parse(line),
                Err(TagLineError::UnknownTag(_))
            ));
        }
    }

    #[test]
    fn invented_tags_are_rejected() {
        assert!(matches!(
            TagLine::parse("COMMON · PLAIN · POETIC"),
            Err(TagLineError::UnknownTag(_))
        ));
        assert!(matches!(
            TagLine::parse("EVERYDAY · PLAIN"),
            Err(TagLineError::UnknownTag(_))
        ));
        assert!(matches!(
            TagLine::parse("COMMON · PLAIN (colloquial)"),
            Err(TagLineError::UnknownStructural(_))
        ));
        assert_eq!(
            TagLine::parse("COMMON · PLAIN · FORMAL"),
            Err(TagLineError::TwoBaselines("PLAIN and FORMAL".into()))
        );
    }

    #[test]
    fn a_duplicated_mark_collapses() {
        assert_eq!(
            round_trip("COMMON · PLAIN · TECHNICAL · TECHNICAL"),
            "COMMON · PLAIN · TECHNICAL"
        );
    }
}
