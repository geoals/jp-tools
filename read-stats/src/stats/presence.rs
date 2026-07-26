//! How much of each inter-line gap counts as reading.
//!
//! This is the rule every other aggregate in [`crate::stats`] credits time
//! through. It lives alone in one module for a reason: while the focus metric
//! ran on the old flat cap and the day totals ran on this, the two disagreed
//! about the same afternoon — a 90-second sentence worked through with four
//! lookups was "reading" to one and "lost focus" to the other.

use super::line::LineEvent;

/// How much of each inter-line gap counts as reading.
///
/// The gap after a line is the time spent reading it, but only some of that
/// time is necessarily *yours* — you may have walked away mid-line. A flat cap
/// was the old answer: credit `min(gap, afk_secs)`. That is right for a gap of
/// 35 seconds and fiction for one of seven minutes, and it charged both the
/// same half-minute, which quietly inflated active time by 22 minutes on
/// 2026-07-19 and deflated that day's speed by 11%.
///
/// So credit what can be *shown*. A lookup or a mined card at time `t` proves
/// you were at the keyboard at `t`, and past the last such proof the line is
/// worth what it would take to read at your uninterrupted pace. Anything
/// beyond that is absence and earns nothing.
pub struct Presence<'a> {
    /// Sorted proofs of presence — lookup and card timestamps, merged.
    marks: &'a [f64],
    /// Chars per second over gaps that needed no adjustment in the first place.
    /// `None` when the stream is too sparse to establish one, in which case
    /// every line falls back to the flat cap.
    pace: Option<f64>,
    afk_secs: f64,
}

impl<'a> Presence<'a> {
    /// `marks` must be sorted. `pace` comes from [`measure_pace`] — passed in
    /// rather than derived here so that every endpoint prices absence against
    /// the *same* window. Deriving it from whatever slice of lines a request
    /// happened to fetch made the dashboard (all history) and the timeline (one
    /// day) disagree about the same day.
    pub fn new(marks: &'a [f64], pace: Option<f64>, afk_secs: f64) -> Self {
        Self {
            marks,
            pace,
            afk_secs,
        }
    }

    /// Credit for the `gap` seconds following `line`. Never exceeds the gap.
    ///
    /// A gap inside the cap is credited whole, exactly as it always was. Most
    /// reading lives here and nothing about it is in doubt — and it *must* pass
    /// through untouched, because pricing every gap at what its line was
    /// "worth" would clip each above-average gap down to average while leaving
    /// the below-average ones alone. That is a systematic shortening dressed up
    /// as a correction; it cost 2026-07-19 42 minutes when only 18 of them were
    /// really absence.
    ///
    /// Past the cap the two cases diverge:
    ///
    /// *Evidence in the gap.* The clock restarts at the last proof of presence
    /// and runs a fresh `afk_secs` from there. A lookup is not instantaneous —
    /// reading the definition is what comes *after* the event fires — so the
    /// grace has to be the same one any line gets. This is what stops a
    /// 45-second dictionary detour being truncated to a flat 30.
    ///
    /// *Nothing in the gap.* Only the line itself can be claimed, at your
    /// uninterrupted pace. A 15-character line earns about four seconds whether
    /// the gap ran 35 seconds or seven minutes; the rest is absence, and the
    /// flat cap paid out half a minute for it regardless.
    pub fn credit(&self, line: &LineEvent, gap: f64) -> f64 {
        if gap <= self.afk_secs {
            return gap;
        }
        match last_mark(self.marks, line.ts, line.ts + gap) {
            Some(ts) => gap.min(ts - line.ts + self.afk_secs),
            None => self.worth(line),
        }
    }

    /// Whether anything in the gap after `line` proves you were at the desk.
    pub fn saw_activity(&self, line: &LineEvent, gap: f64) -> bool {
        has_mark(self.marks, line.ts, line.ts + gap)
    }

    /// What `line` should have taken to read, uninterrupted. Falls back to the
    /// flat cap when the stream is too sparse to have established a pace.
    fn worth(&self, line: &LineEvent) -> f64 {
        match self.pace {
            Some(p) => (line.chars as f64 / p).min(self.afk_secs),
            None => self.afk_secs,
        }
    }
}

/// Reading pace in chars per second, measured over evidence-free gaps at or
/// under the cap — the ones nobody disputes — so it never depends on the credit
/// it goes on to compute. `None` when the stream is too sparse to establish
/// one, which makes every line fall back to the flat cap.
///
/// Feed this the *whole* history rather than a request's slice: it is a
/// property of the reader, and pricing one day's absence against that day's own
/// pace makes two views of the same day disagree.
pub fn measure_pace(lines: &[LineEvent], marks: &[f64], afk_secs: f64) -> Option<f64> {
    let (mut chars, mut secs) = (0i64, 0.0);
    for (k, line) in lines.iter().enumerate() {
        let Some(next) = lines.get(k + 1) else {
            continue;
        };
        let gap = next.ts - line.ts;
        if gap > 0.0 && gap <= afk_secs && !has_mark(marks, line.ts, next.ts) {
            chars += line.chars;
            secs += gap;
        }
    }
    (secs > 0.0 && chars > 0).then(|| chars as f64 / secs)
}

/// Minimum credited time in the window before an effective pace is trusted.
/// Below an hour the ratio is one sitting's worth of noise, and a session
/// duration derived from it would be worse than admitting there isn't one.
const EFFECTIVE_PACE_FLOOR_SECS: f64 = 3600.0;

/// Chars per second *including* everything reading actually costs — the gaps
/// spent in a dictionary, the re-reads, the pauses short enough to still be
/// reading. Total characters over total credited time.
///
/// Deliberately not [`measure_pace`], which measures the opposite quantity: the
/// undisputed sub-cap gaps only, i.e. how fast text goes by when nothing
/// interrupts. That one prices a *gap* and must exclude the interruptions it is
/// deciding about. This one answers "how long did that take me", so it has to
/// include them — using the raw figure here would understate every estimated
/// session by however much the lookups cost.
///
/// `since_ts` bounds it to recent reading: unlike the presence pace, this is
/// used to estimate untimed sessions being logged *now*, and a year-old speed
/// is not the speed they were read at. `None` when the window holds less than
/// [`EFFECTIVE_PACE_FLOOR_SECS`] of reading.
pub fn measure_effective_pace(
    lines: &[LineEvent],
    presence: &Presence,
    session_gap_secs: f64,
    since_ts: f64,
) -> Option<f64> {
    let (mut chars, mut secs) = (0i64, 0.0);
    let mut prev: Option<LineEvent> = None;
    for line in lines {
        if line.ts >= since_ts {
            chars += line.chars;
            if let Some(prev) = prev {
                let gap = line.ts - prev.ts;
                if gap > 0.0 && gap <= session_gap_secs {
                    secs += presence.credit(&prev, gap);
                }
            }
        }
        prev = Some(*line);
    }
    (secs >= EFFECTIVE_PACE_FLOOR_SECS && chars > 0).then(|| chars as f64 / secs)
}

/// Merge lookup, card and #read-action timestamps into one sorted evidence
/// stream. All three are equally proof the reader was at the keyboard.
pub fn presence_marks(lookups: &[f64], cards: &[f64], reader: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(lookups.len() + cards.len() + reader.len());
    out.extend_from_slice(lookups);
    out.extend_from_slice(cards);
    out.extend_from_slice(reader);
    out.sort_by(f64::total_cmp);
    out
}

/// Latest mark in `[from, to)`, if any. `marks` is sorted.
fn last_mark(marks: &[f64], from: f64, to: f64) -> Option<f64> {
    let end = marks.partition_point(|&ts| ts < to);
    marks[..end].last().copied().filter(|&ts| ts >= from)
}

/// Whether any timestamp falls in `[from, to)`. The slice is sorted, so this is
/// a binary search rather than a scan per gap.
pub(crate) fn has_mark(marks: &[f64], from: f64, to: f64) -> bool {
    let at = marks.partition_point(|&ts| ts < from);
    marks.get(at).is_some_and(|&ts| ts < to)
}

/// How many marks fall in `[from, to)`. `marks` is sorted.
pub(crate) fn marks_in(marks: &[f64], from: f64, to: f64) -> i64 {
    let end = marks.partition_point(|&ts| ts < to);
    let start = marks.partition_point(|&ts| ts < from);
    (end - start) as i64
}

#[cfg(test)]
mod tests {
    use super::super::testutil::ev;
    use super::super::timeline::bucket_lines;
    use super::*;

    #[test]
    fn walking_away_earns_the_line_and_nothing_more() {
        // Steady 4s lines establish 25 chars/s, then a seven-minute absence
        // after a 100-char line. That line was worth 4 seconds; the other
        // 416 are absence and must not reach the clock. The flat cap credited
        // a blanket 30s to every such gap, which on 2026-07-19 alone put 22
        // minutes of phantom reading on the day and cost 11% of its speed.
        let lines = [ev(0.0, 100), ev(4.0, 100), ev(8.0, 100), ev(428.0, 100)];
        let p = Presence::new(&[], measure_pace(&lines, &[], 30.0), 30.0);
        let b = bucket_lines(&lines, &[], &p, 600.0, 6000.0);
        assert_eq!(
            b[0].active_secs, 12.0,
            "4 + 4 + the 4 the last line was worth"
        );

        // The cap still bounds a line nobody can price: an enormous line in an
        // enormous gap cannot claim more than the grace.
        assert_eq!(p.credit(&ev(0.0, 100_000), 900.0), 30.0);
        // And a gap shorter than the line's worth is credited whole — the rule
        // never invents time, it only declines to.
        assert_eq!(p.credit(&ev(0.0, 100), 2.0), 2.0);
    }

    #[test]
    fn ordinary_gaps_are_never_repriced() {
        // Gaps within the cap vary either side of the line's "worth" by their
        // nature — that spread *is* the reading. Pricing each at its worth
        // would clip every long one and keep every short one, shortening the
        // day by a quarter while claiming to remove absence. Total credited
        // time across sub-cap gaps must equal the wall clock they span.
        let lines: Vec<_> = (0..40)
            .scan(0.0, |t, i| {
                let line = ev(*t, 20 + (i % 13) * 5);
                *t += 2.0 + (i % 9) as f64 * 3.0; // 2..26s, all inside the cap
                Some(line)
            })
            .collect();
        let p = Presence::new(&[], measure_pace(&lines, &[], 30.0), 30.0);
        let credited: f64 = lines
            .windows(2)
            .map(|w| p.credit(&w[0], w[1].ts - w[0].ts))
            .sum();
        let wall = lines.last().unwrap().ts - lines[0].ts;
        assert_eq!(credited, wall, "no sub-cap gap may be shortened");
    }

    #[test]
    fn presence_survives_a_stream_with_no_measurable_pace() {
        // Every gap is over the cap, so there is nothing to derive a pace from.
        // Rather than divide by zero or credit nothing, fall back to the flat
        // cap — the old behaviour, which is the right thing to degrade to.
        let lines = [ev(0.0, 100), ev(200.0, 100), ev(400.0, 100)];
        let p = Presence::new(&[], measure_pace(&lines, &[], 30.0), 30.0);
        assert_eq!(p.credit(&lines[0], 200.0), 30.0);
    }
}
