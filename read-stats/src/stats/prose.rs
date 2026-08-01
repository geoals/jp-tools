//! How long a work's sentences run.
//!
//! Context for the difficulty figures rather than a finding on its own. A work
//! written in short lines reads nothing like one that narrates in
//! sixty-character sentences, and neither the character count nor the speed on
//! the shelf can tell them apart. The caller states the figures against the
//! rest of the corpus, since a bare median means nothing on its own.

use jp_core::text::{chars::count_chars, sentences::split_sentences};
use serde::Serialize;

/// A work needs this many sentences before its length percentiles say anything
/// about the writing rather than about the last scene read.
const SENTENCE_FLOOR: usize = 200;

#[derive(Debug, Default, Clone, Serialize)]
pub struct ProseStats {
    pub chars: i64,
    pub sentences: usize,
    /// Median sentence length in counted characters, and the 90th percentile —
    /// the long tail is the part a median hides, and it is what makes a work
    /// feel dense even when its average is ordinary.
    pub median_len: Option<f64>,
    pub p90_len: Option<f64>,
}

/// Accumulates sentence lengths as text arrives, so one pass over the stream
/// can feed several of these at once (a work and the rest of the corpus).
#[derive(Default)]
pub struct ProseAcc {
    chars: i64,
    all: Vec<i64>,
}

impl ProseAcc {
    pub fn push(&mut self, text: &str) {
        self.chars += count_chars(text);
        for s in split_sentences(text) {
            let len = count_chars(&s);
            if len > 0 {
                self.all.push(len);
            }
        }
    }

    pub fn finish(mut self) -> ProseStats {
        ProseStats {
            chars: self.chars,
            sentences: self.all.len(),
            median_len: percentile(&mut self.all.clone(), 0.5),
            p90_len: percentile(&mut self.all, 0.9),
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

    #[test]
    fn percentiles_need_enough_sentences() {
        let mut a = ProseAcc::default();
        a.push("一文だけ。");
        let s = a.finish();
        assert_eq!(s.sentences, 1);
        assert_eq!(s.median_len, None);
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
        // `count_chars` excludes the 。, the same rule every other character
        // count on the dashboard is held to.
        assert_eq!(s.median_len, Some(2.0), "短い");
        assert!(
            s.p90_len.unwrap() > s.median_len.unwrap(),
            "a median alone hides the long ones"
        );
    }
}
