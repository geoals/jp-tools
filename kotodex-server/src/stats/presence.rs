//! How much of each inter-line gap counts as reading — the rule every aggregate
//! in [`crate::stats`] credits time through.
//!
//! One module so there can only be one rule. Two of them, and a sentence worked
//! through with four lookups is "reading" to one aggregate and "lost focus" to
//! the next.

use super::line::LineEvent;

/// How much of each inter-line gap counts as reading.
///
/// The gap after a line is time spent reading it, but you may have walked away
/// mid-line. A flat `min(gap, afk_secs)` charges a seven-minute absence the same
/// half-minute as a 35-second pause, which invents reading that never happened.
///
/// So credit what can be shown: a lookup or a mined card at time `t` proves you
/// were at the keyboard at `t`, and past the last such proof the line is worth
/// what it takes to read at your uninterrupted pace. The rest earns nothing.
pub struct Presence<'a> {
    /// Sorted proofs of presence — lookup and card timestamps, merged.
    marks: &'a [f64],
    /// Chars per second over gaps that needed no adjustment. `None` when the
    /// stream is too sparse, in which case every line falls back to the cap.
    pace: Option<f64>,
    afk_secs: f64,
}

impl<'a> Presence<'a> {
    /// `marks` must be sorted. `pace` comes from [`measure_pace`] and is passed
    /// in so every endpoint prices absence against the same window — derived per
    /// request, the dashboard and the timeline disagree about a day.
    pub fn new(marks: &'a [f64], pace: Option<f64>, afk_secs: f64) -> Self {
        Self {
            marks,
            pace,
            afk_secs,
        }
    }

    /// Credit for the `gap` seconds following `line`. Never exceeds the gap.
    ///
    /// A gap inside the cap is credited whole, and must be: pricing every gap
    /// at what its line was "worth" clips the above-average ones to average and
    /// leaves the rest, which shortens the day while claiming to remove absence.
    ///
    /// Past the cap, two cases:
    ///
    /// - **Evidence in the gap** — the clock restarts at the last proof and runs
    ///   a fresh `afk_secs`. Reading a definition happens *after* the lookup
    ///   fires, so a 45-second detour is credited 45, not truncated to 30.
    /// - **Nothing in the gap** — only the line itself is claimed, at your
    ///   uninterrupted pace. A 15-character line earns about four seconds
    ///   whether the gap ran 35 seconds or seven minutes.
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

/// Reading pace in chars per second, over evidence-free gaps at or under the
/// cap so it never depends on the credit it computes. `None` when the stream is
/// too sparse.
///
/// Feed it the *whole* history, not a request's slice — pace is a property of
/// the reader.
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
/// Below an hour the ratio is one sitting's worth of noise.
const EFFECTIVE_PACE_FLOOR_SECS: f64 = 3600.0;

/// Chars per second *including* what reading costs — dictionary gaps, re-reads,
/// short pauses. Total characters over total credited time.
///
/// The opposite quantity to [`measure_pace`], which asks how fast text goes by
/// when nothing interrupts. That one prices a gap, so it excludes the
/// interruptions; this one answers "how long did that take me" and includes
/// them, or it would understate every estimated session by the lookup cost.
///
/// `since_ts` bounds it to recent reading, since it estimates sessions logged
/// *now*. `None` below [`EFFECTIVE_PACE_FLOOR_SECS`] of reading in the window.
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

#[cfg(test)]
mod tests {
    use super::super::testutil::ev;
    use super::super::timeline::bucket_lines;
    use super::*;

    #[test]
    fn walking_away_earns_the_line_and_nothing_more() {
        // Steady 4s lines establish 25 chars/s, then a seven-minute absence
        // after a 100-char line. That line was worth 4 seconds; the other
        // 416 are absence and must not reach the clock.
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
        // day while claiming to remove absence. Total credited time across
        // sub-cap gaps must equal the wall clock they span.
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
        // cap.
        let lines = [ev(0.0, 100), ev(200.0, 100), ev(400.0, 100)];
        let p = Presence::new(&[], measure_pace(&lines, &[], 30.0), 30.0);
        assert_eq!(p.credit(&lines[0], 200.0), 30.0);
    }
}
