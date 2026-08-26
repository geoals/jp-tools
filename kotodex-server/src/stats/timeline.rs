//! One day sliced into fine buckets — the intra-day reading curve.
//!
//! Everything else in [`crate::stats`] aggregates to a day or coarser. This
//! module is the only one that has to place a line's characters and the seconds
//! they cost in the *same* slot, which is why it credits time forward from each
//! line rather than backward from the next one the way [`super::day`] does.

use std::collections::BTreeMap;

use serde::Serialize;

use super::line::LineEvent;
use super::presence::{Presence, has_mark};

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
    /// Characters whose *own* gap contained no lookup — read at uninterrupted
    /// pace. Pairs with `active_secs - lookup_secs`.
    ///
    /// **Both sides have to drop together.** Dividing all `chars` by non-lookup
    /// time credits characters read *during* a lookup to the time that remains,
    /// and in a dense burst the denominator collapses while the numerator does
    /// not — it reported 30k chars/h for reading running at 12k.
    pub clean_chars: i64,
    /// Characters whose own gap *did* contain a lookup. With `clean_chars` these
    /// price the reading embedded in lookup gaps — a gap holds both the line's
    /// reading and the detour, so charging it whole to "looking words up"
    /// overstates the tax.
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
/// up. `presence` decides how much of each gap is credited at all — a lookup
/// late in a long gap now proves you were still there, so a 45-second detour
/// is no longer truncated to the flat 30 the old cap imposed.
pub fn bucket_lines(
    lines: &[LineEvent],
    lookups: &[f64],
    presence: &Presence,
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
                let is_lookup = has_mark(lookups, line.ts, next.ts);
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
                    presence.credit(line, gap),
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
///
/// A bucket index identifies a bucket uniquely only while `bucket_secs` stays
/// at or below the session gap — two sessions are separated by more than that
/// gap, and two instants more than one bucket width apart cannot share a
/// bucket. Callers must hold to that (`day_timeline` clamps for it); the map is
/// keyed by session as well so that violating it misplaces events in a
/// bucket-sized way rather than dropping a whole session's worth into another.
pub fn add_events(buckets: &mut [Bucket], events: &[f64], bucket_secs: f64, field: EventKind) {
    let by_idx: BTreeMap<(i64, usize), usize> = buckets
        .iter()
        .enumerate()
        .map(|(pos, b)| (((b.t / bucket_secs).round() as i64, b.session), pos))
        .collect();
    for &ts in events {
        let idx = (ts / bucket_secs).floor() as i64;
        if let Some((_, &pos)) = by_idx.range((idx, usize::MIN)..=(idx, usize::MAX)).next() {
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

#[cfg(test)]
mod tests {
    use super::super::presence::measure_pace;
    use super::super::session::derive_sessions;
    use super::super::testutil::ev;
    use super::*;

    #[test]
    fn buckets_align_chars_with_the_time_they_cost() {
        // Two lines 90s apart, afk cap 30: the first line's 100 chars and the
        // 30s credited for reading them must land in the same bucket, even
        // though the *next* line is two minutes later.
        let lines = [ev(0.0, 100), ev(90.0, 40)];
        let b = bucket_lines(
            &lines,
            &[],
            &Presence::new(&[], measure_pace(&lines, &[], 30.0), 30.0),
            600.0,
            60.0,
        );
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
        let b = bucket_lines(
            &lines,
            &[],
            &Presence::new(&[], measure_pace(&lines, &[], 30.0), 30.0),
            600.0,
            60.0,
        );
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
        let b = bucket_lines(
            &lines,
            &[],
            &Presence::new(&[], measure_pace(&lines, &[], 30.0), 30.0),
            600.0,
            60.0,
        );
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
        let b = bucket_lines(
            &lines,
            &[],
            &Presence::new(&[], measure_pace(&lines, &[], 30.0), 30.0),
            600.0,
            60.0,
        );
        let sessions = derive_sessions(
            &lines,
            &Presence::new(&[], measure_pace(&lines, &[], 30.0), 30.0),
            600.0,
        );
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
        let b = bucket_lines(
            &lines,
            &[10.0],
            &Presence::new(&[10.0], measure_pace(&lines, &[10.0], 30.0), 30.0),
            600.0,
            6000.0,
        );
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
        assert!(
            raw > effective,
            "a long lookup gap drags the measured rate down"
        );
    }

    #[test]
    fn lookup_overhead_excludes_the_reading_inside_the_gap() {
        // Clean gaps establish the pace: 100 chars per 4s = 25 chars/s. The
        // lookup gap runs 24s and carries 100 chars, so 4s of it was reading
        // the line and only 20s was the dictionary detour. Charging the whole
        // 24s to "looking words up" would overstate it by a fifth.
        let lines = [ev(0.0, 100), ev(4.0, 100), ev(8.0, 100), ev(32.0, 100)];
        let b = bucket_lines(
            &lines,
            &[10.0],
            &Presence::new(&[10.0], measure_pace(&lines, &[10.0], 30.0), 30.0),
            600.0,
            6000.0,
        );
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
        let b = bucket_lines(
            &lines,
            &[25.0],
            &Presence::new(&[25.0], measure_pace(&lines, &[25.0], 30.0), 30.0),
            600.0,
            6000.0,
        );
        let clean_secs = b[0].active_secs - b[0].lookup_secs;
        let raw = b[0].clean_chars as f64 / (clean_secs / 3600.0);
        // Effective over the chars that actually have time attributed to them —
        // which is `clean + lookup`, the trailing line having no gap. Using all
        // `chars` here would divide three lines' characters by two lines' time
        // and report a tax the other way round.
        let timed_chars = b[0].clean_chars + b[0].lookup_chars;
        assert_eq!(
            timed_chars,
            b[0].chars - 100,
            "the trailing line is excluded"
        );
        let effective = timed_chars as f64 / (b[0].active_secs / 3600.0);
        assert_eq!(raw, effective, "no penalty in, no penalty out");
    }

    #[test]
    fn every_char_is_clean_lookup_or_trailing() {
        // The invariant the two speeds rest on: `clean + lookup` is exactly the
        // set of characters that has seconds attributed to it, and what's left
        // over is one line per session — the one with no gap after it.
        let mut ts = 0.0;
        let lines: Vec<_> = (0..120)
            .map(|i| {
                let line = ev(ts, 20 + i % 11);
                ts += match i % 23 {
                    22 => 900.0, // session break
                    11 => 120.0, // over the afk cap
                    _ => 3.0 + (i % 5) as f64 * 4.0,
                };
                line
            })
            .collect();
        let lookups: Vec<f64> = (0..40).map(|i| i as f64 * 37.0 + 1.0).collect();
        let b = bucket_lines(
            &lines,
            &lookups,
            &Presence::new(&lookups, measure_pace(&lines, &lookups, 30.0), 30.0),
            600.0,
            60.0,
        );

        let sum = |f: fn(&Bucket) -> i64| b.iter().map(f).sum::<i64>();
        // A line ends its session when nothing follows it within the gap.
        let trailing: i64 = lines
            .iter()
            .enumerate()
            .filter(|(k, l)| lines.get(k + 1).is_none_or(|n| n.ts - l.ts > 600.0))
            .map(|(_, l)| l.chars)
            .sum();

        assert_eq!(
            sum(|x| x.clean_chars) + sum(|x| x.lookup_chars) + trailing,
            sum(|x| x.chars)
        );
        assert!(
            sum(|x| x.lookup_chars) > 0,
            "the fixture must exercise both sides"
        );
        assert!(sum(|x| x.clean_chars) > 0);
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
        let b = bucket_lines(
            &lines,
            &lookups,
            &Presence::new(&lookups, measure_pace(&lines, &lookups, 30.0), 30.0),
            600.0,
            6000.0,
        );
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
        // A lookup in a gap far longer than the cap: the label can only cover
        // credited time, so subtracting it can never go negative.
        let lines = [ev(0.0, 50), ev(300.0, 50)];
        let b = bucket_lines(
            &lines,
            &[150.0],
            &Presence::new(&[150.0], measure_pace(&lines, &[150.0], 30.0), 30.0),
            600.0,
            600.0,
        );
        let total_active: f64 = b.iter().map(|x| x.active_secs).sum();
        let total_lookup: f64 = b.iter().map(|x| x.lookup_secs).sum();
        // Present at 150s, so the credit runs to 180 — the flat cap paid 30 for
        // the same evidence, throwing away two and a half minutes it could see.
        assert_eq!(total_active, 180.0);
        assert_eq!(total_lookup, 180.0, "the whole credited gap was a lookup");
        assert!(
            total_lookup <= total_active,
            "the label is time, not extra time"
        );
    }

    #[test]
    fn events_land_in_their_bucket_and_outsiders_are_dropped() {
        let lines = [ev(0.0, 10), ev(30.0, 10), ev(90.0, 10)];
        let mut b = bucket_lines(
            &lines,
            &[],
            &Presence::new(&[], measure_pace(&lines, &[], 30.0), 30.0),
            600.0,
            60.0,
        );
        // 10s and 40s → bucket 0; 95s → bucket 1; 9000s → no session, dropped.
        add_events(&mut b, &[10.0, 40.0, 95.0, 9000.0], 60.0, EventKind::Lookup);
        assert_eq!(b[0].lookups, 2);
        assert_eq!(b[1].lookups, 1);
        assert_eq!(b.iter().map(|x| x.lookups).sum::<i64>(), 3);
    }
}
