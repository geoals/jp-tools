//! How much of the reading was people talking, and at what speed.
//!
//! The classification itself is free — Japanese prose brackets speech with
//! 「」, so [`crate::dialogue`] partitions a line's characters with nothing but
//! bracket depth. What this module adds is the aggregation, and the one rule
//! that makes the speed comparison honest: characters are counted over *every*
//! line, but seconds are counted only over lines that are wholly one kind.
//! See [`Side`] for why those two populations must differ.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::Serialize;

use super::day::date_key;
use super::line::LineEvent;
use super::presence::{Presence, marks_in};
use crate::dialogue::Kind;

/// One side of the dialogue/narration split.
///
/// `chars` and the `timed_*` pairs answer different questions and are counted
/// over different lines on purpose, so never divide one by the other.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct Side {
    /// Every character of this kind, from every line — the exact half of the
    /// partition, including the ones on mixed lines. This is what "what share
    /// of my reading was dialogue" is asking for, so it must not skip lines.
    pub chars: i64,
    /// Characters on lines that were *wholly* this kind, with the seconds
    /// credited to the gaps after them. Speed comes from this pair alone.
    ///
    /// Mixed lines are dropped from both halves rather than assigned to the
    /// larger side. A gap is one undivided span of time — whatever the reader
    /// spent on `「そうか」と彼は言った` cannot be split into a dialogue part
    /// and a narration part after the fact, and charging it whole to the
    /// majority kind would bias the comparison by exactly the amount the
    /// comparison is trying to measure. On the current corpus that discards
    /// 30 lines in 5183.
    pub timed_chars: i64,
    pub timed_secs: f64,
    /// The subset of `timed_*` whose gap contained no lookup: reading speed on
    /// text the reader already knew. Both halves drop together for the same
    /// reason `Bucket::clean_chars` exists — dividing all characters by only
    /// the uninterrupted seconds inflates the rate without bound.
    pub clean_chars: i64,
    pub clean_secs: f64,
    /// Lookups fired inside those gaps. Against `timed_chars` this is the
    /// unknown-word rate for this kind of text.
    pub lookups: i64,
    pub lines: i64,
}

impl Side {
    /// Chars per hour as actually read, lookups and all.
    pub fn speed(&self) -> Option<f64> {
        (self.timed_secs > 0.0 && self.timed_chars > 0)
            .then(|| self.timed_chars as f64 * 3600.0 / self.timed_secs)
    }

    /// Chars per hour over gaps that held no lookup.
    pub fn clean_speed(&self) -> Option<f64> {
        (self.clean_secs > 0.0 && self.clean_chars > 0)
            .then(|| self.clean_chars as f64 * 3600.0 / self.clean_secs)
    }

    /// Lookups per 1000 characters. Floored like `lookup_rate` elsewhere: below
    /// a few hundred characters this is mostly noise.
    pub fn lookups_per_1k(&self) -> Option<f64> {
        (self.timed_chars >= 500).then(|| self.lookups as f64 * 1000.0 / self.timed_chars as f64)
    }

    fn add(&mut self, other: &Side) {
        self.chars += other.chars;
        self.timed_chars += other.timed_chars;
        self.timed_secs += other.timed_secs;
        self.clean_chars += other.clean_chars;
        self.clean_secs += other.clean_secs;
        self.lookups += other.lookups;
        self.lines += other.lines;
    }
}

/// A day's reading split into speech and prose.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct DialogueDay {
    pub dialogue: Side,
    pub narration: Side,
}

impl DialogueDay {
    /// Share of characters that were spoken, 0..1. `None` when the day has no
    /// classified text (manual sessions, or rows logged before text was kept).
    pub fn share(&self) -> Option<f64> {
        let total = self.dialogue.chars + self.narration.chars;
        (total > 0).then(|| self.dialogue.chars as f64 / total as f64)
    }

    pub fn add(&mut self, other: &DialogueDay) {
        self.dialogue.add(&other.dialogue);
        self.narration.add(&other.narration);
    }
}

/// Split a line stream into per-day dialogue and narration totals.
///
/// Unlike [`super::day::aggregate_line_days`], a gap's seconds go to the day of
/// its *earlier* line, not its later one — the line that generated the gap is
/// the one whose kind decides where those seconds belong, and splitting the two
/// across a day boundary would put a line's characters and their cost on
/// different days. The difference is one gap per rollover and these totals are
/// not reconciled against the day figures anyway (see `Side::timed_chars`,
/// which deliberately counts fewer lines).
pub fn aggregate_dialogue_days(
    lines: &[LineEvent],
    lookups: &[f64],
    presence: &Presence,
    session_gap_secs: f64,
    rollover_hour: i64,
    tz_offset_secs: i64,
) -> BTreeMap<NaiveDate, DialogueDay> {
    let mut out: BTreeMap<NaiveDate, DialogueDay> = BTreeMap::new();

    for (k, line) in lines.iter().enumerate() {
        if !line.classified {
            continue;
        }
        let day = out
            .entry(date_key(line.ts, rollover_hour, tz_offset_secs))
            .or_default();
        day.dialogue.chars += line.dialogue_chars;
        day.narration.chars += line.narration_chars();

        // Time needs a gap to measure and a pure line to attribute it to.
        let Some(kind) = line.kind() else { continue };
        let Some(next) = lines.get(k + 1) else {
            continue;
        };
        let gap = next.ts - line.ts;
        if gap <= 0.0 || gap > session_gap_secs {
            continue;
        }
        let side = match kind {
            Kind::Dialogue => &mut day.dialogue,
            Kind::Narration => &mut day.narration,
        };
        let credit = presence.credit(line, gap);
        side.timed_chars += line.chars;
        side.timed_secs += credit;
        side.lines += 1;
        let hits = marks_in(lookups, line.ts, next.ts);
        if hits == 0 {
            side.clean_chars += line.chars;
            side.clean_secs += credit;
        } else {
            side.lookups += hits;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::testutil::{ev, prose, spoken};
    use super::*;

    /// Build a day's worth of dialogue aggregates with no lookups and a flat
    /// 10-second beat, so every gap is credited whole and speeds are exact.
    fn dialogue_day(lines: &[LineEvent], lookups: &[f64]) -> DialogueDay {
        let presence = Presence::new(lookups, None, 30.0);
        aggregate_dialogue_days(lines, lookups, &presence, 600.0, 4, 0)
            .into_values()
            .fold(DialogueDay::default(), |mut acc, d| {
                acc.add(&d);
                acc
            })
    }

    #[test]
    fn share_counts_every_character_including_mixed_lines() {
        // The share is the whole point of the feature, so it must be over all
        // text — not just the pure lines the speed comparison can use.
        let lines = [
            spoken(0.0, 30),
            prose(10.0, 10),
            LineEvent {
                ts: 20.0,
                chars: 20,
                dialogue_chars: 15,
                classified: true,
            },
            spoken(30.0, 40.0 as i64),
        ];
        let day = dialogue_day(&lines, &[]);
        assert_eq!(day.dialogue.chars, 30 + 15 + 40);
        assert_eq!(day.narration.chars, 10 + 5);
        assert_eq!(day.share().unwrap(), 85.0 / 100.0);
    }

    #[test]
    fn a_mixed_line_contributes_no_time_to_either_side() {
        // Its 20 chars are split for the share above, but its gap is one span
        // of time that belongs to both kinds at once. Charging it to the
        // majority side would bias the speed comparison by exactly the thing
        // being compared, so it is dropped from `timed_*` entirely.
        let lines = [
            LineEvent {
                ts: 0.0,
                chars: 20,
                dialogue_chars: 15,
                classified: true,
            },
            spoken(10.0, 30),
            prose(20.0, 30),
        ];
        let day = dialogue_day(&lines, &[]);
        assert_eq!(day.dialogue.lines, 1, "only the pure spoken line is timed");
        assert_eq!(day.dialogue.timed_chars, 30);
        assert_eq!(day.narration.lines, 0, "the last line has no gap after it");
        assert_eq!(day.dialogue.timed_secs, 10.0);
    }

    #[test]
    fn speed_is_measured_per_kind() {
        // Dialogue at 10 chars/s, narration at 5 — the real corpus shows a gap
        // this shaped, and it has to survive being pooled into one day.
        let mut lines = Vec::new();
        let mut ts = 0.0;
        for _ in 0..10 {
            lines.push(spoken(ts, 100));
            ts += 10.0;
            lines.push(prose(ts, 50));
            ts += 10.0;
        }
        let day = dialogue_day(&lines, &[]);
        assert_eq!(day.dialogue.speed().unwrap(), 100.0 * 360.0);
        assert_eq!(day.narration.speed().unwrap(), 50.0 * 360.0);
    }

    #[test]
    fn a_lookup_moves_its_gap_out_of_the_clean_speed_on_the_right_side() {
        // Both halves of the clean pair have to drop together: the chars read
        // across a lookup gap leave with the seconds they cost. Otherwise a
        // narration-heavy lookup burst reports narration reading faster than
        // it does — the same explosion `raw_speed_cannot_explode_in_a_lookup_burst`
        // pins for the day timeline.
        let lines = [
            prose(0.0, 100),
            prose(10.0, 100),
            prose(20.0, 100),
            spoken(30.0, 100),
        ];
        let lookups = [5.0, 7.0]; // two lookups, both in the first gap
        let day = dialogue_day(&lines, &lookups);

        assert_eq!(day.narration.timed_chars, 300);
        assert_eq!(
            day.narration.clean_chars, 200,
            "the first line's chars leave"
        );
        assert_eq!(day.narration.clean_secs, 20.0, "and so do its seconds");
        assert_eq!(day.narration.lookups, 2, "both count, not just the gap");
        assert_eq!(day.dialogue.lookups, 0);
        assert!(
            day.narration.clean_speed().unwrap() == day.narration.speed().unwrap(),
            "even pace here, so removing a whole gap changes nothing"
        );
    }

    #[test]
    fn unclassified_lines_are_left_out_rather_than_called_narration() {
        // Rows stored before text was kept have dialogue_chars 0, which is
        // indistinguishable from prose. Counting them would silently drag the
        // share toward narration for every day of the old history.
        let lines = [
            ev(0.0, 100),
            ev(10.0, 100),
            prose(20.0, 50),
            prose(30.0, 50),
        ];
        let day = dialogue_day(&lines, &[]);
        assert_eq!(day.narration.chars, 100, "only the two classified lines");
        assert_eq!(day.dialogue.chars, 0);
        assert_eq!(day.share().unwrap(), 0.0);
    }

    #[test]
    fn a_day_with_no_classified_text_has_no_share() {
        let day = dialogue_day(&[ev(0.0, 100), ev(10.0, 100)], &[]);
        assert!(day.share().is_none(), "no text to split — not 0% dialogue");
    }
}
