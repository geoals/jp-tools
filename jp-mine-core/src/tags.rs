//! The canonical two-axis tag rubric (FAMILIARITY + FLAVOR) shared by every LLM
//! call that emits it: the CompactDef gloss ([`crate::compactdef`]) and
//! read-stats' explain button. Both used to carry their own paraphrase of these
//! definitions and had already drifted apart; this is the single source of truth
//! so a wording change lands everywhere at once.
//!
//! FAMILIARITY uses the sharpened definitions: the axis turns on the single
//! question "can you be certain EVERY native adult recognizes it?", with COMMON
//! vs UNCOMMON split by active-vs-passive vocabulary and RARE as the first tier
//! where universal recognition can no longer be assumed.
//!
//! It is rated on the spelling the reader actually met, which is why both
//! callers send a surface form and never a dictionary headword: 饐える is a rare
//! kanji, すえた臭い is a phrase people say, and the card is about the second.

/// The FAMILIARITY axis — one tier, recognition-on-sight across the population.
pub const FAMILIARITY_RUBRIC: &str = "\
FAMILIARITY (exactly one) — recognition-on-sight across the native adult \
population (NOT frequency, NOT whether they say it). The axis turns on ONE \
question: can you be certain EVERY native adult recognizes it?\n\
- CORE — every native, from childhood.\n\
- COMMON — every native adult knows it, and for most it is ACTIVE vocabulary \
(they would use it themselves).\n\
- UNCOMMON — essentially every native adult still RECOGNIZES it, but for a large \
portion it is PASSIVE only (known, but they would not produce it).\n\
- RARE — the first tier where you CANNOT be certain every adult knows it. Many \
do, but a large share of such words are recognized mainly by people who read.\n\
- OBSCURE — you can assume non-readers do NOT know it, and even among active \
readers only a portion recognize it.\n\
A transparent compound of common parts is understood first-encounter (等価値 = \
等価+価値) → COMMON or higher. You are biased by written frequency: spoken and \
colloquial words are more familiar than their rarity in print suggests.\n\
Rate the word AS IT IS WRITTEN in front of you. A word usually met in kana is \
as familiar as the kana makes it, however rare the kanji spelling would be.";

/// The FLAVOR axis — one baseline formality plus up to two independent marks.
pub const FLAVOR_RUBRIC: &str = "\
FLAVOR (1-3) — if you SAY it in the wrong room, how do you sound. Emit exactly \
one baseline formality, then add marks only when they carry an independent, \
equally-important warning:\n\
- baseline: SLANG / PLAIN (safe anywhere — always shown) / FORMAL (stiff if \
casual; fine in formal speech or writing) / LITERARY (writing-only; theatrical \
if spoken).\n\
- marks: TECHNICAL, RELIGIOUS, HONORIFIC, HUMBLE, DIALECT, ARCHAIC, VULGAR, \
DEROGATORY, CHILDISH.\n\
Tag the IN-SENTENCE sense; other senses don't count (joking 成仏 = PLAIN, not \
RELIGIOUS). A word can be marked in origin but plain in use — tag current usage, \
not etymology.";

/// The five FAMILIARITY tiers, most familiar first. Index is the tier order.
pub const FAMILIARITY: [&str; 5] = ["CORE", "COMMON", "UNCOMMON", "RARE", "OBSCURE"];

/// The four baseline formalities. Exactly one is required.
pub const BASELINES: [&str; 4] = ["SLANG", "PLAIN", "FORMAL", "LITERARY"];

/// The independent marks, added to a baseline. Order in the rendered line
/// follows this array, not the order the model happened to emit them in.
pub const MARKS: [&str; 9] = [
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
    #[error("no familiarity tier: line starts with {0:?}")]
    NoFamiliarity(String),
    #[error("no baseline formality (one of SLANG/PLAIN/FORMAL/LITERARY)")]
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
    pub familiarity: &'static str,
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
            .filter(|t| !t.is_empty());

        let first = tokens.next().ok_or(TagLineError::Empty)?;
        let familiarity = *FAMILIARITY
            .iter()
            .find(|f| f.eq_ignore_ascii_case(first))
            .ok_or_else(|| TagLineError::NoFamiliarity(first.to_string()))?;

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
        write!(f, "{} · {}", self.familiarity, self.baseline)?;
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
            "UNCOMMON · FORMAL · HONORIFIC",
            "CORE · PLAIN (mimetic)",
            "RARE · LITERARY",
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
            round_trip("UNCOMMON PLAIN · DIALECT"),
            "UNCOMMON · PLAIN · DIALECT"
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
    #[test]
    fn a_missing_baseline_is_rejected() {
        assert_eq!(
            TagLine::parse("UNCOMMON · TECHNICAL"),
            Err(TagLineError::NoBaseline)
        );
        assert_eq!(TagLine::parse("COMMON"), Err(TagLineError::NoBaseline));
    }

    #[test]
    fn invented_tags_are_rejected() {
        assert!(matches!(
            TagLine::parse("COMMON · PLAIN · POETIC"),
            Err(TagLineError::UnknownTag(_))
        ));
        assert!(matches!(
            TagLine::parse("EVERYDAY · PLAIN"),
            Err(TagLineError::NoFamiliarity(_))
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
