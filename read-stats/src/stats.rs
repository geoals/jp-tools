//! Pure derivation logic: raw line events → sessions, active time, per-day
//! aggregates. All functions take thresholds and a fixed tz offset as
//! parameters so they stay deterministic and unit-testable.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::Serialize;

#[derive(Debug, Clone, Copy)]
pub struct LineEvent {
    pub ts: f64,
    pub chars: i64,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Session {
    pub start_ts: f64,
    pub end_ts: f64,
    pub chars: i64,
    pub active_secs: f64,
    pub lines: i64,
}

/// Per-day totals for one source bucket.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct DayBucket {
    pub chars: i64,
    pub active_secs: f64,
}

/// Split a time-ordered line stream into sessions and derive active reading
/// time. Each inter-line gap credits reading time capped at `afk_secs` (a
/// longer gap means the reader walked away mid-session); a gap above
/// `session_gap_secs` closes the session. A lone line yields a session with
/// zero active time — credit comes from gaps, not line count.
pub fn derive_sessions(lines: &[LineEvent], afk_secs: f64, session_gap_secs: f64) -> Vec<Session> {
    let mut out: Vec<Session> = Vec::new();
    for line in lines {
        match out.last_mut() {
            Some(s) if line.ts - s.end_ts <= session_gap_secs => {
                s.active_secs += (line.ts - s.end_ts).min(afk_secs);
                s.end_ts = line.ts;
                s.chars += line.chars;
                s.lines += 1;
            }
            _ => out.push(Session {
                start_ts: line.ts,
                end_ts: line.ts,
                chars: line.chars,
                active_secs: 0.0,
                lines: 1,
            }),
        }
    }
    out
}

/// Local calendar date a timestamp belongs to, with the day boundary shifted
/// to `rollover_hour` (reading at 02:30 counts toward the previous day when
/// the rollover is 04:00).
pub fn date_key(ts: f64, rollover_hour: i64, tz_offset_secs: i64) -> NaiveDate {
    let shifted = ts as i64 + tz_offset_secs - rollover_hour * 3600;
    NaiveDate::from_ymd_opt(1970, 1, 1).unwrap() + chrono::Duration::days(shifted.div_euclid(86400))
}

/// Inverse of `date_key`: epoch timestamp at which `date` begins.
pub fn day_start_ts(date: NaiveDate, rollover_hour: i64, tz_offset_secs: i64) -> f64 {
    let days = (date - NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()).num_days();
    (days * 86400 + rollover_hour * 3600 - tz_offset_secs) as f64
}

/// Aggregate a time-ordered line stream into per-day char/active-time totals.
/// Chars go to the day of their line; gap credit goes to the day of the gap's
/// *later* line (same capping rules as `derive_sessions`).
pub fn aggregate_line_days(
    lines: &[LineEvent],
    afk_secs: f64,
    session_gap_secs: f64,
    rollover_hour: i64,
    tz_offset_secs: i64,
) -> BTreeMap<NaiveDate, DayBucket> {
    let mut out: BTreeMap<NaiveDate, DayBucket> = BTreeMap::new();
    let mut prev_ts: Option<f64> = None;
    for line in lines {
        let day = out
            .entry(date_key(line.ts, rollover_hour, tz_offset_secs))
            .or_default();
        day.chars += line.chars;
        if let Some(prev) = prev_ts {
            let gap = line.ts - prev;
            if gap > 0.0 && gap <= session_gap_secs {
                day.active_secs += gap.min(afk_secs);
            }
        }
        prev_ts = Some(line.ts);
    }
    out
}

/// One fixed-width slice of a day's reading. Deliberately finer than anything
/// worth plotting: the client smooths these to whatever granularity it's asked
/// for, so moving the smoothing control never refetches.
#[derive(Debug, Clone, Serialize)]
pub struct Bucket {
    /// Bucket start, epoch seconds.
    pub t: f64,
    /// Index of the session this bucket belongs to. Buckets never span
    /// sessions, so the client can break the line between them instead of
    /// drawing a slope through a dinner break.
    pub session: usize,
    pub chars: i64,
    /// Characters whose *own* gap contained no lookup — i.e. read at
    /// uninterrupted pace. Pairs with `active_secs - lookup_secs`.
    ///
    /// Speed on the text itself has to drop both sides together. Dividing all
    /// `chars` by non-lookup time only would credit characters read *during* a
    /// lookup to the time that remains, and in a dense lookup burst the
    /// denominator collapses while the numerator doesn't — which reported
    /// 30k chars/h for reading that was actually running at 12k.
    pub clean_chars: i64,
    /// Characters whose own gap *did* contain a lookup. With `clean_chars`
    /// these price the reading embedded in lookup gaps: a gap holds both the
    /// line's reading and the dictionary detour, so charging the whole gap to
    /// "looking words up" overstates it by whatever that line would have cost
    /// at clean pace.
    pub lookup_chars: i64,
    pub active_secs: f64,
    /// The part of `active_secs` spent inside a gap that contained a Yomitan
    /// lookup. Always ≤ `active_secs`: it is a *label* on credited time, not
    /// extra time.
    pub lookup_secs: f64,
    pub lookups: i64,
    pub cards: i64,
}

fn empty_bucket(session: usize, idx: i64, bucket_secs: f64) -> Bucket {
    Bucket {
        t: idx as f64 * bucket_secs,
        session,
        chars: 0,
        clean_chars: 0,
        lookup_chars: 0,
        active_secs: 0.0,
        lookup_secs: 0.0,
        lookups: 0,
        cards: 0,
    }
}

/// Spread `dur` seconds of reading credit starting at `start` across the
/// buckets it covers, so a credit straddling a boundary lands on both sides
/// instead of being dumped whole into one.
fn spread_credit(
    out: &mut BTreeMap<(usize, i64), Bucket>,
    session: usize,
    start: f64,
    dur: f64,
    bucket_secs: f64,
    is_lookup: bool,
) {
    let end = start + dur;
    let mut t = start;
    while t < end {
        let idx = (t / bucket_secs).floor() as i64;
        let boundary = (idx + 1) as f64 * bucket_secs;
        let chunk = boundary.min(end) - t;
        let bucket = out
            .entry((session, idx))
            .or_insert_with(|| empty_bucket(session, idx, bucket_secs));
        bucket.active_secs += chunk;
        if is_lookup {
            bucket.lookup_secs += chunk;
        }
        t = boundary;
    }
}

/// Whether any lookup timestamp falls in `[from, to)`. `lookups` is sorted, so
/// this is a binary search rather than a scan per gap.
fn gap_has_lookup(lookups: &[f64], from: f64, to: f64) -> bool {
    let at = lookups.partition_point(|&ts| ts < from);
    lookups.get(at).is_some_and(|&ts| ts < to)
}

/// Slice a time-ordered line stream into per-bucket chars and active time.
///
/// Time is credited to the interval *after* each line — `[ts, ts + min(gap,
/// afk)]` — not to the following line's bucket the way the per-day aggregates
/// do it. The gap after a line is the time spent reading that line, so this is
/// what puts a line's characters and the seconds they cost in the same bucket.
/// At day granularity the difference is invisible; at one minute it's the
/// difference between a speed curve and noise.
///
/// Buckets are zero-filled within each session: a minute inside a session with
/// no lines is real (a pause, a lookup that ran long), and dropping it would
/// silently compress the time axis.
///
/// `lookups` (sorted) labels each gap that contains one, so the caller can
/// separate speed on the text from speed including the cost of looking words
/// up. Note the afk cap truncates a long lookup, so the labelled time is a
/// lower bound on what a lookup actually cost.
pub fn bucket_lines(
    lines: &[LineEvent],
    lookups: &[f64],
    afk_secs: f64,
    session_gap_secs: f64,
    bucket_secs: f64,
) -> Vec<Bucket> {
    let mut out: BTreeMap<(usize, i64), Bucket> = BTreeMap::new();
    let mut session = 0usize;

    for (k, line) in lines.iter().enumerate() {
        if k > 0 && line.ts - lines[k - 1].ts > session_gap_secs {
            session += 1;
        }
        let idx = (line.ts / bucket_secs).floor() as i64;
        out.entry((session, idx))
            .or_insert_with(|| empty_bucket(session, idx, bucket_secs))
            .chars += line.chars;

        if let Some(next) = lines.get(k + 1) {
            let gap = next.ts - line.ts;
            if gap > 0.0 && gap <= session_gap_secs {
                let is_lookup = gap_has_lookup(lookups, line.ts, next.ts);
                // These characters were read across this gap, so they follow
                // the gap's own classification.
                let bucket = out
                    .entry((session, idx))
                    .or_insert_with(|| empty_bucket(session, idx, bucket_secs));
                if is_lookup {
                    bucket.lookup_chars += line.chars;
                } else {
                    bucket.clean_chars += line.chars;
                }
                spread_credit(
                    &mut out,
                    session,
                    line.ts,
                    gap.min(afk_secs),
                    bucket_secs,
                    is_lookup,
                );
            }
        }
    }

    // Zero-fill each session's interior.
    let mut filled: Vec<Bucket> = Vec::new();
    let mut prev: Option<(usize, i64)> = None;
    for (&(session, idx), bucket) in &out {
        if let Some((prev_session, prev_idx)) = prev
            && prev_session == session
        {
            for gap_idx in (prev_idx + 1)..idx {
                filled.push(empty_bucket(session, gap_idx, bucket_secs));
            }
        }
        filled.push(bucket.clone());
        prev = Some((session, idx));
    }
    filled
}

/// Add point events (lookup or card timestamps) to the buckets holding them.
/// Events outside every session are dropped: with no reading time around them
/// there is no per-hour rate they could belong to.
pub fn add_events(buckets: &mut [Bucket], events: &[f64], bucket_secs: f64, field: EventKind) {
    let by_idx: BTreeMap<i64, usize> = buckets
        .iter()
        .enumerate()
        .map(|(pos, b)| ((b.t / bucket_secs).round() as i64, pos))
        .collect();
    for &ts in events {
        let idx = (ts / bucket_secs).floor() as i64;
        if let Some(&pos) = by_idx.get(&idx) {
            match field {
                EventKind::Lookup => buckets[pos].lookups += 1,
                EventKind::Card => buckets[pos].cards += 1,
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum EventKind {
    Lookup,
    Card,
}

#[derive(Debug, Clone)]
pub struct WorkLine {
    pub ts: f64,
    pub chars: i64,
    pub work: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct WorkAgg {
    pub chars: i64,
    pub active_secs: f64,
    pub first_ts: f64,
    pub last_ts: f64,
}

/// Aggregate a time-ordered line stream per work title. Gap credit follows the
/// same capping rules as sessions and goes to the *later* line's work, so a
/// mid-session switch splits time at the switch point.
pub fn aggregate_works(
    lines: &[WorkLine],
    afk_secs: f64,
    session_gap_secs: f64,
) -> BTreeMap<Option<String>, WorkAgg> {
    let mut out: BTreeMap<Option<String>, WorkAgg> = BTreeMap::new();
    let mut prev_ts: Option<f64> = None;
    for line in lines {
        let agg = out.entry(line.work.clone()).or_insert_with(|| WorkAgg {
            first_ts: line.ts,
            ..Default::default()
        });
        agg.chars += line.chars;
        agg.last_ts = line.ts;
        if let Some(prev) = prev_ts {
            let gap = line.ts - prev;
            if gap > 0.0 && gap <= session_gap_secs {
                agg.active_secs += gap.min(afk_secs);
            }
        }
        prev_ts = Some(line.ts);
    }
    out
}

/// A gap longer than this counts as an interruption and breaks a flow stretch.
/// Above `afk_secs` (you weren't reading at pace) but well under
/// `session_gap_secs` (you didn't leave) — the range where a distraction lives.
pub const INTERRUPTION_SECS: f64 = 60.0;

/// How continuous the reading was, as opposed to how much of it there was.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct FocusDay {
    /// Gap time credited as reading (each gap capped at afk_secs).
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

/// Per-day focus from the raw line stream. Deliberately *not* derived from the
/// capped active time alone: the afk cap is exactly what hides fragmentation,
/// so this keeps the uncapped span beside it and reports the ratio.
///
/// Gaps over `session_gap_secs` are excluded — that's leaving, not being
/// distracted, and it already ends the session.
pub fn aggregate_focus_days(
    lines: &[LineEvent],
    afk_secs: f64,
    session_gap_secs: f64,
    rollover_hour: i64,
    tz_offset_secs: i64,
) -> BTreeMap<NaiveDate, FocusDay> {
    let mut out: BTreeMap<NaiveDate, FocusDay> = BTreeMap::new();
    let mut prev_ts: Option<f64> = None;
    let mut stretch = 0.0;

    for line in lines {
        let date = date_key(line.ts, rollover_hour, tz_offset_secs);
        if let Some(prev) = prev_ts {
            let gap = line.ts - prev;
            if gap > 0.0 && gap <= session_gap_secs {
                let day = out.entry(date).or_default();
                day.active_secs += gap.min(afk_secs);
                day.span_secs += gap;
                if gap > INTERRUPTION_SECS {
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
        prev_ts = Some(line.ts);
    }
    out
}

#[derive(Debug, Clone, Copy)]
pub struct PauseInterval {
    pub start_ts: f64,
    /// None = still paused (open interval extends to now).
    pub end_ts: Option<f64>,
}

pub fn is_paused(ts: f64, pauses: &[PauseInterval]) -> bool {
    pauses
        .iter()
        .any(|p| ts >= p.start_ts && p.end_ts.is_none_or(|end| ts < end))
}

/// Current and best streak of days meeting `floor_secs` of active time.
/// The current streak counts back from `today`; an unmet *today* doesn't
/// break it (the day isn't over yet), but an unmet yesterday does.
pub fn streaks(
    days: &BTreeMap<NaiveDate, DayBucket>,
    floor_secs: f64,
    today: NaiveDate,
) -> (i64, i64) {
    let met = |d: NaiveDate| days.get(&d).is_some_and(|b| b.active_secs >= floor_secs);

    let mut current = 0i64;
    let mut cursor = if met(today) {
        today
    } else {
        today - chrono::Duration::days(1)
    };
    while met(cursor) {
        current += 1;
        cursor -= chrono::Duration::days(1);
    }

    let mut best = 0i64;
    let mut run = 0i64;
    let mut prev: Option<NaiveDate> = None;
    for (&date, bucket) in days {
        if bucket.active_secs < floor_secs {
            prev = None;
            run = 0;
            continue;
        }
        run = match prev {
            Some(p) if date - p == chrono::Duration::days(1) => run + 1,
            _ => 1,
        };
        best = best.max(run);
        prev = Some(date);
    }
    (current, best.max(current))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(ts: f64, chars: i64) -> LineEvent {
        LineEvent { ts, chars }
    }

    #[test]
    fn focus_ratio_separates_steady_from_fragmented() {
        // Steady: 20 lines, 5s apart. Every gap is a normal reading beat.
        let steady: Vec<_> = (0..20).map(|i| ev(i as f64 * 5.0, 30)).collect();
        let day = *aggregate_focus_days(&steady, 20.0, 600.0, 4, 0)
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
        let day = *aggregate_focus_days(&frag, 20.0, 600.0, 4, 0)
            .values()
            .next()
            .unwrap();
        // 4, not 5: the last increment trails the final line, so it never
        // becomes a gap between two lines.
        assert_eq!(day.interruptions, 4, "each 300s gap is one interruption");
        // Span carries the full 300s detours; active credits only 20s of each.
        assert!(day.span_secs > day.active_secs * 3.0);
        assert!(day.ratio().unwrap() < 0.3);
        assert_eq!(day.longest_stretch_secs, 15.0, "interruptions break the run");
    }

    #[test]
    fn focus_ignores_between_session_gaps() {
        // A 30-minute break is leaving, not a distraction: it must not count as
        // an interruption or inflate the span.
        let lines = [ev(0.0, 10), ev(5.0, 10), ev(1805.0, 10), ev(1810.0, 10)];
        let days = aggregate_focus_days(&lines, 20.0, 600.0, 4, 0);
        let total: f64 = days.values().map(|d| d.span_secs).sum();
        let interruptions: i64 = days.values().map(|d| d.interruptions).sum();
        assert_eq!(total, 10.0, "only the two in-session gaps count");
        assert_eq!(interruptions, 0);
    }

    #[test]
    fn focus_ratio_needs_enough_span() {
        let short = [ev(0.0, 10), ev(5.0, 10)];
        let day = *aggregate_focus_days(&short, 20.0, 600.0, 4, 0)
            .values()
            .next()
            .unwrap();
        assert_eq!(day.ratio(), None, "5s of span can't support a ratio");
    }

    #[test]
    fn sessions_split_on_gap_and_cap_afk() {
        let lines = [ev(0.0, 10), ev(30.0, 20), ev(330.0, 5), ev(1000.0, 7)];
        // afk cap 120s, session gap 600s: the 300s gap stays in-session but
        // credits only 120s; the 670s gap starts a new session.
        let sessions = derive_sessions(&lines, 120.0, 600.0);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].chars, 35);
        assert_eq!(sessions[0].lines, 3);
        assert_eq!(sessions[0].active_secs, 30.0 + 120.0);
        assert_eq!(sessions[0].end_ts, 330.0);
        assert_eq!(sessions[1].chars, 7);
        assert_eq!(sessions[1].active_secs, 0.0);
    }

    #[test]
    fn buckets_align_chars_with_the_time_they_cost() {
        // Two lines 90s apart, afk cap 30: the first line's 100 chars and the
        // 30s credited for reading them must land in the same bucket, even
        // though the *next* line is two minutes later.
        let lines = [ev(0.0, 100), ev(90.0, 40)];
        let b = bucket_lines(&lines, &[], 30.0, 600.0, 60.0);
        assert_eq!(b.len(), 2, "60s and 120s buckets, zero-filled between");
        assert_eq!((b[0].chars, b[0].active_secs), (100, 30.0));
        assert_eq!(b[1].chars, 40, "second line's bucket");
        assert_eq!(b[1].active_secs, 0.0, "no line after it to credit a gap");
    }

    #[test]
    fn bucket_credit_splits_across_a_boundary() {
        // A 20s gap starting 10s before the boundary: 10s each side, not 20 on
        // one. Total credit is preserved.
        let lines = [ev(50.0, 10), ev(70.0, 10)];
        let b = bucket_lines(&lines, &[], 30.0, 600.0, 60.0);
        assert_eq!(b[0].active_secs, 10.0);
        assert_eq!(b[1].active_secs, 10.0);
        let total: f64 = b.iter().map(|x| x.active_secs).sum();
        assert_eq!(total, 20.0);
    }

    #[test]
    fn buckets_zero_fill_inside_a_session_but_not_between() {
        // A 5-min lull (under the 600s session gap) fills; a 20-min break
        // starts a new session and leaves no buckets in between.
        let lines = [ev(0.0, 10), ev(300.0, 10), ev(1600.0, 10)];
        let b = bucket_lines(&lines, &[], 30.0, 600.0, 60.0);
        let s0: Vec<_> = b.iter().filter(|x| x.session == 0).collect();
        assert_eq!(s0.len(), 6, "minutes 0..5 all present");
        assert!(s0[1..5].iter().all(|x| x.chars == 0), "lull is zero-filled");
        assert_eq!(b.iter().filter(|x| x.session == 1).count(), 1);
        assert_eq!(b.len(), 7, "nothing drawn across the break");
    }

    #[test]
    fn bucket_totals_match_session_totals() {
        // Whatever the bucketing does to placement, it must not create or lose
        // chars or seconds relative to the session derivation.
        // Varying gaps, including some over the afk cap and two over the
        // session gap, so every branch of both derivations is exercised.
        let mut ts = 0.0;
        let lines: Vec<_> = (0..200)
            .map(|i| {
                let line = ev(ts, 20 + i % 11);
                ts += match i % 17 {
                    16 => 900.0, // session break
                    13 => 120.0, // over the afk cap
                    _ => 4.0 + (i % 7) as f64 * 3.0,
                };
                line
            })
            .collect();
        let b = bucket_lines(&lines, &[], 30.0, 600.0, 60.0);
        let sessions = derive_sessions(&lines, 30.0, 600.0);
        let bucket_chars: i64 = b.iter().map(|x| x.chars).sum();
        let session_chars: i64 = sessions.iter().map(|s| s.chars).sum();
        assert_eq!(bucket_chars, session_chars);
        let bucket_secs: f64 = b.iter().map(|x| x.active_secs).sum();
        let session_secs: f64 = sessions.iter().map(|s| s.active_secs).sum();
        assert!((bucket_secs - session_secs).abs() < 1e-6);
    }

    #[test]
    fn lookup_gaps_label_the_time_they_consumed() {
        // Realistic shape: clean gaps are short (4s) and the gap holding a
        // lookup is long (24s), which is where the penalty actually lives —
        // in the measured stream, lookup gaps run a median 21s against 3s.
        let lines = [ev(0.0, 100), ev(4.0, 100), ev(8.0, 100), ev(32.0, 100)];
        let b = bucket_lines(&lines, &[10.0], 30.0, 600.0, 6000.0);
        assert_eq!(b.len(), 1, "all inside one bucket");
        assert_eq!(b[0].active_secs, 32.0);
        assert_eq!(b[0].lookup_secs, 24.0, "only the gap containing the lookup");

        // The 100 chars read across the lookup gap drop out of the clean side
        // along with their 24 seconds; the final line has no gap, so its chars
        // count toward the total but toward neither rate's denominator.
        assert_eq!(b[0].chars, 400);
        assert_eq!(b[0].clean_chars, 200);

        let effective = b[0].chars as f64 / (b[0].active_secs / 3600.0);
        let raw = b[0].clean_chars as f64 / ((b[0].active_secs - b[0].lookup_secs) / 3600.0);
        assert_eq!(effective, 45000.0);
        assert_eq!(raw, 90000.0, "clean chars over clean seconds");
        assert!(raw > effective, "a long lookup gap drags the measured rate down");
    }

    #[test]
    fn lookup_overhead_excludes_the_reading_inside_the_gap() {
        // Clean gaps establish the pace: 100 chars per 4s = 25 chars/s. The
        // lookup gap runs 24s and carries 100 chars, so 4s of it was reading
        // the line and only 20s was the dictionary detour. Charging the whole
        // 24s to "looking words up" would overstate it by a fifth.
        let lines = [ev(0.0, 100), ev(4.0, 100), ev(8.0, 100), ev(32.0, 100)];
        let b = bucket_lines(&lines, &[10.0], 30.0, 600.0, 6000.0);
        let x = &b[0];
        assert_eq!((x.clean_chars, x.lookup_chars), (200, 100));

        let clean_rate = x.clean_chars as f64 / (x.active_secs - x.lookup_secs);
        assert_eq!(clean_rate, 25.0, "chars per second at uninterrupted pace");

        let baseline = x.lookup_chars as f64 / clean_rate;
        assert_eq!(baseline, 4.0, "the reading embedded in the lookup gap");
        assert_eq!(x.lookup_secs - baseline, 20.0, "actual lookup overhead");
    }

    #[test]
    fn raw_speed_matches_effective_when_lookups_cost_nothing() {
        // Same chars in the same time whether or not a lookup happened: the two
        // rates must agree. A formula that removed the seconds but kept the
        // characters would report a gap here where there is none.
        let lines = [ev(0.0, 100), ev(20.0, 100), ev(40.0, 100)];
        let b = bucket_lines(&lines, &[25.0], 30.0, 600.0, 6000.0);
        let clean_secs = b[0].active_secs - b[0].lookup_secs;
        let raw = b[0].clean_chars as f64 / (clean_secs / 3600.0);
        // Effective over the chars that actually have time attributed to them.
        let timed_chars = b[0].chars - 100; // the trailing line has no gap
        let effective = timed_chars as f64 / (b[0].active_secs / 3600.0);
        assert_eq!(raw, effective, "no penalty in, no penalty out");
    }

    #[test]
    fn raw_speed_cannot_explode_in_a_lookup_burst() {
        // The bug this guards: dividing *all* chars by only the non-lookup
        // seconds. Here every gap but one contains a lookup, so that denominator
        // is tiny while the numerator is not — the old formula reported a wild
        // multiple of the true pace. Clean-over-clean stays put.
        let lines: Vec<_> = (0..21).map(|i| ev(i as f64 * 20.0, 100)).collect();
        // A lookup in every gap except the first.
        let lookups: Vec<f64> = (1..20).map(|i| i as f64 * 20.0 + 5.0).collect();
        let b = bucket_lines(&lines, &lookups, 30.0, 600.0, 6000.0);
        let bucket = &b[0];

        let clean_secs = bucket.active_secs - bucket.lookup_secs;
        assert_eq!(clean_secs, 20.0, "only the first gap is clean");
        assert_eq!(bucket.clean_chars, 100, "and only its line's chars");

        let effective = bucket.chars as f64 / (bucket.active_secs / 3600.0);
        let raw = bucket.clean_chars as f64 / (clean_secs / 3600.0);
        let bugged = bucket.chars as f64 / (clean_secs / 3600.0);

        assert_eq!(raw, 18000.0);
        assert_eq!(bugged, 378_000.0, "what the old formula produced");
        assert!(
            raw < effective * 2.0,
            "raw {raw} must stay in the neighbourhood of effective {effective}"
        );
    }

    #[test]
    fn lookup_time_never_exceeds_active_time() {
        // A lookup in a gap far longer than the afk cap: the label can only
        // cover credited time, so subtracting it can never go negative.
        let lines = [ev(0.0, 50), ev(300.0, 50)];
        let b = bucket_lines(&lines, &[150.0], 30.0, 600.0, 600.0);
        let total_active: f64 = b.iter().map(|x| x.active_secs).sum();
        let total_lookup: f64 = b.iter().map(|x| x.lookup_secs).sum();
        assert_eq!(total_active, 30.0, "afk cap truncates the 300s gap");
        assert_eq!(total_lookup, 30.0, "the whole credited gap was a lookup");
        assert!(total_lookup <= total_active);
    }

    #[test]
    fn events_land_in_their_bucket_and_outsiders_are_dropped() {
        let lines = [ev(0.0, 10), ev(30.0, 10), ev(90.0, 10)];
        let mut b = bucket_lines(&lines, &[], 30.0, 600.0, 60.0);
        // 10s and 40s → bucket 0; 95s → bucket 1; 9000s → no session, dropped.
        add_events(&mut b, &[10.0, 40.0, 95.0, 9000.0], 60.0, EventKind::Lookup);
        assert_eq!(b[0].lookups, 2);
        assert_eq!(b[1].lookups, 1);
        assert_eq!(b.iter().map(|x| x.lookups).sum::<i64>(), 3);
    }

    #[test]
    fn date_key_applies_rollover_and_offset() {
        // local midnight 2026-07-19 at UTC+2 = 2026-07-18 22:00 UTC
        let midnight = 1784412000.0;
        let offset = 7200;
        // 03:00 local, rollover 04 → previous day
        assert_eq!(
            date_key(midnight + 3.0 * 3600.0, 4, offset),
            NaiveDate::from_ymd_opt(2026, 7, 18).unwrap()
        );
        // 05:00 local, rollover 04 → same day
        assert_eq!(
            date_key(midnight + 5.0 * 3600.0, 4, offset),
            NaiveDate::from_ymd_opt(2026, 7, 19).unwrap()
        );
        // round trip
        let d = NaiveDate::from_ymd_opt(2026, 7, 19).unwrap();
        assert_eq!(date_key(day_start_ts(d, 4, offset), 4, offset), d);
    }

    #[test]
    fn aggregate_credits_gap_to_later_line_day() {
        let offset = 0;
        let d1 = day_start_ts(NaiveDate::from_ymd_opt(2026, 7, 18).unwrap(), 0, offset);
        let d2 = day_start_ts(NaiveDate::from_ymd_opt(2026, 7, 19).unwrap(), 0, offset);
        // one line late on day 1, one early on day 2, 60s apart across midnight
        let lines = [ev(d2 - 30.0, 10), ev(d2 + 30.0, 20), ev(d2 + 90.0, 5)];
        let days = aggregate_line_days(&lines, 120.0, 600.0, 0, offset);
        let day1 = days[&NaiveDate::from_ymd_opt(2026, 7, 18).unwrap()];
        let day2 = days[&NaiveDate::from_ymd_opt(2026, 7, 19).unwrap()];
        assert_eq!(day1.chars, 10);
        assert_eq!(day1.active_secs, 0.0);
        assert_eq!(day2.chars, 25);
        assert_eq!(day2.active_secs, 120.0);
        assert!(d1 < d2);
    }

    #[test]
    fn paused_intervals_cover_lines() {
        let pauses = [
            PauseInterval { start_ts: 100.0, end_ts: Some(200.0) },
            PauseInterval { start_ts: 500.0, end_ts: None },
        ];
        assert!(!is_paused(50.0, &pauses));
        assert!(is_paused(100.0, &pauses));
        assert!(is_paused(150.0, &pauses));
        assert!(!is_paused(200.0, &pauses)); // end is exclusive
        assert!(!is_paused(300.0, &pauses));
        assert!(is_paused(9999.0, &pauses)); // open interval
    }

    #[test]
    fn works_split_at_switch_point() {
        let w = |ts: f64, chars: i64, work: &str| WorkLine {
            ts,
            chars,
            work: (!work.is_empty()).then(|| work.to_string()),
        };
        let lines = [
            w(0.0, 10, "A"),
            w(30.0, 10, "A"),
            w(60.0, 10, "B"), // switch: this gap credits B
            w(90.0, 10, "B"),
            w(1000.0, 5, ""), // unlabeled, new session (gap > 600)
        ];
        let works = aggregate_works(&lines, 120.0, 600.0);
        let a = &works[&Some("A".to_string())];
        let b = &works[&Some("B".to_string())];
        assert_eq!((a.chars, a.active_secs), (20, 30.0));
        assert_eq!((b.chars, b.active_secs), (20, 60.0));
        assert_eq!(works[&None].chars, 5);
        assert_eq!(works[&None].active_secs, 0.0);
        assert_eq!(a.first_ts, 0.0);
        assert_eq!(b.last_ts, 90.0);
    }

    #[test]
    fn streaks_current_and_best() {
        let mut days = BTreeMap::new();
        let d = |s: &str| s.parse::<NaiveDate>().unwrap();
        for (date, secs) in [
            ("2026-07-10", 4000.0),
            ("2026-07-11", 4000.0),
            ("2026-07-12", 4000.0),
            // gap on the 13th
            ("2026-07-14", 4000.0),
            ("2026-07-15", 4000.0),
            ("2026-07-16", 1000.0), // under floor
            ("2026-07-17", 4000.0),
            ("2026-07-18", 4000.0),
        ] {
            days.insert(
                d(date),
                DayBucket {
                    chars: 1,
                    active_secs: secs,
                },
            );
        }
        // today (19th) not yet met: streak still counts back from yesterday
        let (current, best) = streaks(&days, 3600.0, d("2026-07-19"));
        assert_eq!(current, 2);
        assert_eq!(best, 3);
        // an unmet yesterday breaks it
        let (current, _) = streaks(&days, 3600.0, d("2026-07-20"));
        assert_eq!(current, 0);
    }
}
