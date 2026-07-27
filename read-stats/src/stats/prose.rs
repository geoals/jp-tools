//! What a work's prose is like: how much of it is spoken, and how long its
//! sentences run.
//!
//! Context for the difficulty figures rather than a finding on its own. A work
//! that is mostly dialogue in short lines reads nothing like one that narrates
//! in sixty-character sentences, and neither the character count nor the speed
//! on the shelf can tell them apart.
//!
//! Two figures, both stated against the rest of the corpus by the caller — a
//! bare "38% dialogue" means nothing without knowing that your reading averages
//! 52%.

use jp_core::text::{chars::count_chars, dialogue, sentences::split_sentences};
use serde::Serialize;

/// Below this share of dialogue characters, a work is treated as not marking
/// speech at all rather than as almost pure narration.
///
/// Not every writer brackets dialogue: some VNs run speech into the prose, and
/// a logged article never brackets anything. Reporting those as "100%
/// narration" would be a measurement of the punctuation dressed up as a
/// measurement of the writing. Ten percent because the slack has to cover more
/// than a stray quoted phrase: a work whose speech is genuinely bracketed runs
/// far above it (素晴らしき日々 is at 69%), while one that only brackets the
/// occasional shout or title lands in the low single digits, and there is
/// nothing in between worth reporting a share for.
const BRACKET_FLOOR: f64 = 0.10;

/// A work needs this many sentences before its length percentiles say anything
/// about the writing rather than about the last scene read.
const SENTENCE_FLOOR: usize = 200;

#[derive(Debug, Default, Clone, Serialize)]
pub struct ProseStats {
    pub chars: i64,
    pub dialogue_chars: i64,
    /// Whether this text marks its speech with 「」 at all. When false the
    /// dialogue share is not reported: see [`BRACKET_FLOOR`].
    pub brackets_dialogue: bool,
    pub sentences: usize,
    /// Median sentence length in counted characters, and the 90th percentile —
    /// the long tail is the part a median hides, and it is what makes a work
    /// feel dense even when its average is ordinary.
    pub median_len: Option<f64>,
    pub p90_len: Option<f64>,
    /// The same median, split by which side of the 「」 the sentence sat on.
    /// Only *pure* lines contribute, following `dialogue::Split::kind` — a
    /// mixed line's sentences cannot be attributed without re-splitting them,
    /// and the whole point here is the contrast between the two registers.
    pub dialogue_median_len: Option<f64>,
    pub narration_median_len: Option<f64>,
}

/// Accumulates sentence lengths as text arrives, so one pass over the stream
/// can feed several of these at once (a work and the rest of the corpus).
#[derive(Default)]
pub struct ProseAcc {
    chars: i64,
    dialogue_chars: i64,
    all: Vec<i64>,
    spoken: Vec<i64>,
    narrated: Vec<i64>,
}

impl ProseAcc {
    pub fn push(&mut self, text: &str) {
        let split = dialogue::split(text);
        self.chars += split.total();
        self.dialogue_chars += split.dialogue;

        let side = split.kind();
        for s in split_sentences(text) {
            let len = count_chars(&s);
            if len == 0 {
                continue;
            }
            self.all.push(len);
            match side {
                Some(dialogue::Kind::Dialogue) => self.spoken.push(len),
                Some(dialogue::Kind::Narration) => self.narrated.push(len),
                None => {}
            }
        }
    }

    pub fn finish(mut self) -> ProseStats {
        let share = self.dialogue_chars as f64 / self.chars.max(1) as f64;
        ProseStats {
            chars: self.chars,
            dialogue_chars: self.dialogue_chars,
            brackets_dialogue: share >= BRACKET_FLOOR,
            sentences: self.all.len(),
            median_len: percentile(&mut self.all, 0.5),
            p90_len: percentile(&mut self.all.clone(), 0.9),
            dialogue_median_len: percentile(&mut self.spoken, 0.5),
            narration_median_len: percentile(&mut self.narrated, 0.5),
        }
    }
}

/// `None` below [`SENTENCE_FLOOR`]: a percentile over forty sentences is a
/// description of one scene, and printing it beside a corpus average invites
/// reading a difference that is noise.
fn percentile(values: &mut Vec<i64>, q: f64) -> Option<f64> {
    if values.len() < SENTENCE_FLOOR {
        return None;
    }
    values.sort_unstable();
    let idx = ((values.len() - 1) as f64 * q).round() as usize;
    Some(values[idx] as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc(lines: &[&str]) -> ProseStats {
        let mut a = ProseAcc::default();
        // Enough repetitions to clear the sentence floor, since the floor is
        // the thing that decides whether a percentile is reported at all.
        for _ in 0..SENTENCE_FLOOR {
            for l in lines {
                a.push(l);
            }
        }
        a.finish()
    }

    #[test]
    fn dialogue_share_and_sentence_lengths() {
        let s = acc(&["「短い」", "長い夜のあとに朝が来た。"]);
        assert!(s.brackets_dialogue);
        // 「」 are not counted characters, so the share is over the text itself.
        assert!(s.dialogue_chars > 0 && s.dialogue_chars < s.chars);
        assert_eq!(s.dialogue_median_len, Some(2.0), "短い");
        // 11, not 12: `count_chars` excludes the 。, the same rule every other
        // character count on the dashboard is held to.
        assert_eq!(s.narration_median_len, Some(11.0), "長い夜のあとに朝が来た");
    }

    #[test]
    fn text_that_never_brackets_speech_reports_no_split() {
        // An article, or a VN that runs speech into the prose. Reporting this
        // as 100% narration would measure the punctuation, not the writing.
        let s = acc(&["今日は晴れだ。明日は雨らしい。"]);
        assert!(!s.brackets_dialogue);
        assert_eq!(s.dialogue_chars, 0);
        assert!(s.median_len.is_some(), "length still says something");
    }

    #[test]
    fn a_stray_quoted_phrase_does_not_make_it_bracketed() {
        // One 「」 in a wall of narration is a quoted title, not speech.
        let mut a = ProseAcc::default();
        for _ in 0..SENTENCE_FLOOR {
            a.push("そこには長い長い物語がずっと書かれていたのだった。");
        }
        a.push("「あ」");
        let s = a.finish();
        assert!(!s.brackets_dialogue);
    }

    #[test]
    fn percentiles_need_enough_sentences() {
        let mut a = ProseAcc::default();
        a.push("一文だけ。");
        let s = a.finish();
        assert_eq!(s.sentences, 1);
        assert_eq!(s.median_len, None);
        assert_eq!(s.dialogue_median_len, None);
    }

    #[test]
    fn the_tail_is_reported_apart_from_the_median() {
        let mut a = ProseAcc::default();
        for _ in 0..SENTENCE_FLOOR {
            a.push("短い。");
        }
        for _ in 0..(SENTENCE_FLOOR / 4) {
            a.push("とてもとても長い文章がここにずっと続いていくのである。");
        }
        let s = a.finish();
        assert!(
            s.p90_len.unwrap() > s.median_len.unwrap(),
            "a median alone hides the long ones"
        );
    }
}
