//! How *continuous* the reading was, as opposed to how much of it there was.
//!
//! Two hours unbroken and two hours in twelve fragments are the same number of
//! minutes and not the same afternoon. Active time cannot tell them apart —
//! the AFK cap is exactly what hides fragmentation — so this keeps the uncapped
//! wall-clock span beside the credited time and reports the ratio.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::Serialize;

use super::day::date_key;
use super::line::LineEvent;
use super::presence::Presence;

/// A gap longer than this counts as an interruption and breaks a flow stretch.
/// Above `afk_secs` (you weren't reading at pace) but well under
/// `session_gap_secs` (you didn't leave) — the range where a distraction lives.
pub const INTERRUPTION_SECS: f64 = 60.0;

/// How continuous the reading was, as opposed to how much of it there was.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct FocusDay {
    /// Gap time credited as reading, by the same `Presence` rule as everything
    /// else — while this ran on a flat cap and the rest did not, a sentence
    /// worked through with four lookups read as a minute of lost focus.
    pub active_secs: f64,
    /// Wall-clock time inside sessions — every gap, uncapped.
    pub span_secs: f64,
    /// Longest run with no gap over INTERRUPTION_SECS.
    pub longest_stretch_secs: f64,
    pub interruptions: i64,
}

impl FocusDay {
    /// Share of at-desk time actually spent reading at pace. 1.0 = every gap
    /// was a normal reading beat; 0.5 = half the session went elsewhere.
    /// None until there's enough span for the ratio to mean anything.
    pub fn ratio(&self) -> Option<f64> {
        (self.span_secs >= 600.0).then(|| self.active_secs / self.span_secs)
    }
}

/// Per-day focus from the raw line stream. Gaps over `session_gap_secs` are
/// excluded — that is leaving, not being distracted, and it already ends the
/// session.
pub fn aggregate_focus_days(
    lines: &[LineEvent],
    presence: &Presence,
    session_gap_secs: f64,
    rollover_hour: i64,
    tz_offset_secs: i64,
) -> BTreeMap<NaiveDate, FocusDay> {
    let mut out: BTreeMap<NaiveDate, FocusDay> = BTreeMap::new();
    let mut prev: Option<LineEvent> = None;
    let mut stretch = 0.0;

    for line in lines {
        let date = date_key(line.ts, rollover_hour, tz_offset_secs);
        if let Some(prev) = prev {
            let gap = line.ts - prev.ts;
            if gap > 0.0 && gap <= session_gap_secs {
                let day = out.entry(date).or_default();
                day.active_secs += presence.credit(&prev, gap);
                day.span_secs += gap;
                // A long gap only breaks the run if nothing in it says you were
                // there. Ninety seconds spent working through one sentence with
                // four lookups is hard reading, not a distraction — counting it
                // as one cut a 98-minute unbroken stretch in half.
                if gap > INTERRUPTION_SECS && !presence.saw_activity(&prev, gap) {
                    day.interruptions += 1;
                    stretch = 0.0;
                } else {
                    stretch += gap;
                    day.longest_stretch_secs = day.longest_stretch_secs.max(stretch);
                }
            } else {
                // Session boundary: the next line starts a fresh stretch.
                stretch = 0.0;
                out.entry(date).or_default();
            }
        }
        prev = Some(*line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::presence::measure_pace;
    use super::super::testutil::ev;
    use super::*;

    #[test]
    fn focus_does_not_punish_looking_words_up() {
        // A hard sentence worked through with lookups: 90 seconds on one line,
        // proof of presence all the way. That is not lost focus, it is reading.
        // While this metric ran on the flat cap and everything else did not, a
        // day like 2026-07-20 read 97.3% focused, and every one of the 17 gaps
        // behind the missing 2.7% turned out to hold lookups.
        // Ten minutes of steady 5s beats, then one line held for 90 seconds.
        let steady: Vec<_> = (0..=120).map(|i| ev(i as f64 * 5.0, 20)).collect();
        let mut worked = steady.clone();
        worked.push(ev(690.0, 20));
        let marks = [605.0, 625.0, 645.0, 665.0]; // four lookups inside the 90s
        let p = Presence::new(&marks, measure_pace(&worked, &marks, 30.0), 30.0);
        let day = *aggregate_focus_days(&worked, &p, 600.0, 4, 0)
            .values()
            .next()
            .unwrap();
        assert_eq!(
            day.active_secs, day.span_secs,
            "every second was accounted for"
        );
        assert_eq!(
            day.ratio(),
            Some(1.0),
            "a dictionary detour is not a distraction"
        );
        // And it doesn't break the run either, though it is over INTERRUPTION_SECS.
        assert_eq!(day.interruptions, 0);
        assert_eq!(day.longest_stretch_secs, 690.0, "one unbroken stretch");

        // The same shape with an eight-minute silence and nothing to explain it
        // still scores badly, which is what the metric is for.
        let mut away = steady;
        away.push(ev(1100.0, 20));
        let bare = Presence::new(&[], measure_pace(&away, &[], 30.0), 30.0);
        let day = *aggregate_focus_days(&away, &bare, 600.0, 4, 0)
            .values()
            .next()
            .unwrap();
        assert!(
            day.ratio().unwrap() < 0.6,
            "unexplained absence still shows"
        );
    }

    #[test]
    fn focus_ratio_separates_steady_from_fragmented() {
        // Steady: 20 lines, 5s apart. Every gap is a normal reading beat.
        let steady: Vec<_> = (0..20).map(|i| ev(i as f64 * 5.0, 30)).collect();
        let day = *aggregate_focus_days(
            &steady,
            &Presence::new(&[], measure_pace(&steady, &[], 20.0), 20.0),
            600.0,
            4,
            0,
        )
        .values()
        .next()
        .unwrap();
        assert_eq!(day.active_secs, day.span_secs, "no gap exceeded the cap");
        assert_eq!(day.interruptions, 0);
        assert_eq!(day.longest_stretch_secs, 95.0);

        // Fragmented: same 20 lines but every fourth gap is a 5-minute detour.
        let mut ts = 0.0;
        let mut frag = Vec::new();
        for i in 0..20 {
            frag.push(ev(ts, 30));
            ts += if i % 4 == 3 { 300.0 } else { 5.0 };
        }
        let day = *aggregate_focus_days(
            &frag,
            &Presence::new(&[], measure_pace(&frag, &[], 20.0), 20.0),
            600.0,
            4,
            0,
        )
        .values()
        .next()
        .unwrap();
        // 4, not 5: the last increment trails the final line, so it never
        // becomes a gap between two lines.
        assert_eq!(day.interruptions, 4, "each 300s gap is one interruption");
        // Span carries the full 300s detours; active credits only 20s of each.
        assert!(day.span_secs > day.active_secs * 3.0);
        assert!(day.ratio().unwrap() < 0.3);
        assert_eq!(
            day.longest_stretch_secs, 15.0,
            "interruptions break the run"
        );
    }

    #[test]
    fn focus_ignores_between_session_gaps() {
        // A 30-minute break is leaving, not a distraction: it must not count as
        // an interruption or inflate the span.
        let lines = [ev(0.0, 10), ev(5.0, 10), ev(1805.0, 10), ev(1810.0, 10)];
        let days = aggregate_focus_days(
            &lines,
            &Presence::new(&[], measure_pace(&lines, &[], 20.0), 20.0),
            600.0,
            4,
            0,
        );
        let total: f64 = days.values().map(|d| d.span_secs).sum();
        let interruptions: i64 = days.values().map(|d| d.interruptions).sum();
        assert_eq!(total, 10.0, "only the two in-session gaps count");
        assert_eq!(interruptions, 0);
    }

    #[test]
    fn focus_ratio_needs_enough_span() {
        let short = [ev(0.0, 10), ev(5.0, 10)];
        let day = *aggregate_focus_days(
            &short,
            &Presence::new(&[], measure_pace(&short, &[], 20.0), 20.0),
            600.0,
            4,
            0,
        )
        .values()
        .next()
        .unwrap();
        assert_eq!(day.ratio(), None, "5s of span can't support a ratio");
    }
}
