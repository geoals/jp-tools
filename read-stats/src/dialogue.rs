//! Splitting a line into spoken dialogue and narration.
//!
//! Japanese prose marks speech with corner brackets — 「…」 for a line of
//! dialogue, 『…』 for quoted-within-quoted and for titles — so the raw text a
//! hook captures already carries the distinction, for free, with no parsing
//! beyond bracket depth. That makes "how much of what I read was people
//! talking" a derivable statistic rather than something to annotate by hand.
//!
//! The split is by *character*, not by line, so `「そうか」と彼は言った` is
//! counted as three dialogue characters and the rest narration. Counting whole
//! lines would have to round that case one way or the other; counting
//! characters partitions [`crate::charcount::count_chars`] exactly, which is
//! the property the aggregates lean on:
//!
//! ```text
//! dialogue(line) + narration(line) == count_chars(line)
//! ```
//!
//! The brackets themselves are not counted either way — they fall outside the
//! `charcount` allowlist, so they never inflate one side.
//!
//! Only the corner brackets count as speech. Double quotes (“…”) appear in VN
//! prose for emphasis and for quoting a phrase rather than a speaker, so
//! treating them as dialogue would file narration under speech; parentheses
//! (（…）) are usually inner monologue, which is neither.

use crate::charcount::is_counted;

/// A line's characters, partitioned.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Split {
    pub dialogue: i64,
    pub narration: i64,
}

impl Split {
    pub fn total(&self) -> i64 {
        self.dialogue + self.narration
    }

    /// Whether the line is wholly one kind. Speed is measured over these only —
    /// a mixed line's characters split cleanly but the seconds after it cannot,
    /// since a gap is one span of time and belongs to whatever was being read
    /// across it. See [`crate::stats::aggregate_dialogue_days`].
    pub fn kind(&self) -> Option<Kind> {
        match (self.dialogue > 0, self.narration > 0) {
            (true, false) => Some(Kind::Dialogue),
            (false, true) => Some(Kind::Narration),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dialogue,
    Narration,
}

/// A gap longer than this closes any bracket left open by the previous line.
///
/// Depth genuinely carries across lines: a hook fires per text box, and a long
/// speech is broken across several of them, leaving the first line's 「
/// unclosed until the last. Resetting per line would file every continuation as
/// narration (on the current corpus, 38 rows — small, but wrong in exactly the
/// place the metric is about). Carrying it forever is the opposite risk: one
/// dropped 」 anywhere in a menu or a name-entry screen would recolor the rest
/// of the session as speech. Five minutes is far past any real continuation and
/// far short of a session, so it bounds the damage to one scene.
pub const CARRY_GAP_SECS: f64 = 300.0;

/// Bracket depth carried across consecutive lines. Feed it a work's lines in
/// timestamp order and call [`Scanner::reset`] when the stream breaks.
#[derive(Debug, Default)]
pub struct Scanner {
    depth: u32,
}

impl Scanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget any unclosed bracket.
    pub fn reset(&mut self) {
        self.depth = 0;
    }

    /// Partition `text`, carrying bracket depth in from the previous line and
    /// out to the next.
    pub fn scan(&mut self, text: &str) -> Split {
        let mut out = Split::default();
        for c in text.chars() {
            match c {
                '「' | '『' => {
                    self.depth += 1;
                }
                '」' | '』' => {
                    self.depth = self.depth.saturating_sub(1);
                }
                _ if is_counted(c) => {
                    if self.depth > 0 {
                        out.dialogue += 1;
                    } else {
                        out.narration += 1;
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// Partition one standalone line, with no carried context.
pub fn split(text: &str) -> Split {
    Scanner::new().scan(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::charcount::count_chars;

    #[test]
    fn plain_speech_is_all_dialogue() {
        let s = split("「ねえ、聞いてる？」");
        assert_eq!(
            s,
            Split {
                dialogue: 6,
                narration: 0
            }
        );
        assert_eq!(s.kind(), Some(Kind::Dialogue));
    }

    #[test]
    fn plain_prose_is_all_narration() {
        let s = split("由岐との会話……。");
        assert_eq!(
            s,
            Split {
                dialogue: 0,
                narration: 6
            }
        );
        assert_eq!(s.kind(), Some(Kind::Narration));
    }

    #[test]
    fn a_quote_inside_prose_splits_by_character() {
        // The case whole-line classification cannot represent.
        let s = split("「そうか」と彼は言った");
        assert_eq!(
            s,
            Split {
                dialogue: 3,
                narration: 6
            }
        );
        assert_eq!(s.kind(), None, "mixed lines are excluded from speed");
    }

    #[test]
    fn the_split_always_partitions_the_char_count() {
        // The invariant every aggregate depends on: neither side can invent or
        // lose a character relative to what the day totals are built from.
        for text in [
            "「ねえ、聞いてる？」",
            "「そうか」と彼は言った",
            "……そうか。",
            "「なんだよ」「うるせぇ」",
            "『銀河鉄道の夜』を読んだ",
            "「開き直るなアホ……」",
            "ＡＢ12ab「ｱｲｳ」",
            "",
        ] {
            let s = split(text);
            assert_eq!(s.total(), count_chars(text), "mismatch on {text:?}");
        }
    }

    #[test]
    fn nested_brackets_stay_dialogue_throughout() {
        // 『』 inside 「」 is still someone talking; depth must not fall to zero
        // at the inner close and file the tail as narration.
        let s = split("「彼は『行く』と言った」");
        assert_eq!(
            s,
            Split {
                dialogue: 8,
                narration: 0
            }
        );
    }

    #[test]
    fn a_speech_broken_across_lines_stays_dialogue() {
        // A long line is hooked as several text boxes, so the 「 opens on one
        // and the 」 closes on a later one. Scanned independently the middle
        // and the tail would read as narration.
        let mut sc = Scanner::new();
        assert_eq!(
            sc.scan("「父さん……"),
            Split {
                dialogue: 3,
                narration: 0
            }
        );
        assert_eq!(
            sc.scan("この考え方正させろよ！」"),
            Split {
                dialogue: 10,
                narration: 0
            }
        );
        assert_eq!(
            sc.scan("彼は黙った。"),
            Split {
                dialogue: 0,
                narration: 5
            }
        );
    }

    #[test]
    fn a_stray_close_bracket_cannot_drive_depth_negative() {
        // Saturating subtraction, so a dropped 「 doesn't leave the scanner
        // owing a close bracket and mark real speech as narration.
        let mut sc = Scanner::new();
        assert_eq!(
            sc.scan("そうか」"),
            Split {
                dialogue: 0,
                narration: 3
            }
        );
        assert_eq!(
            sc.scan("「行こう」"),
            Split {
                dialogue: 3,
                narration: 0
            }
        );
    }

    #[test]
    fn reset_closes_a_bracket_left_hanging() {
        let mut sc = Scanner::new();
        sc.scan("「まだ喋ってる");
        sc.reset();
        assert_eq!(
            sc.scan("地の文だ"),
            Split {
                dialogue: 0,
                narration: 4
            }
        );
    }
}
