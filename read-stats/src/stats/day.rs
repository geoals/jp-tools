//! The calendar boundary, per-day totals, and streaks.
//!
//! A "day" here is not a UTC day or even a local midnight-to-midnight day: it
//! runs from `rollover_hour` to `rollover_hour`, so reading past midnight
//! counts toward the day it felt like. [`date_key`] and [`day_start_ts`] are
//! inverses and are the only two places that boundary is defined.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::Serialize;

use super::line::LineEvent;
use super::presence::Presence;

/// Per-day totals for one source bucket.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct DayBucket {
    pub chars: i64,
    pub active_secs: f64,
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
/// *later* line (same crediting rules as `derive_sessions`).
pub fn aggregate_line_days(
    lines: &[LineEvent],
    presence: &Presence,
    session_gap_secs: f64,
    rollover_hour: i64,
    tz_offset_secs: i64,
) -> BTreeMap<NaiveDate, DayBucket> {
    let mut out: BTreeMap<NaiveDate, DayBucket> = BTreeMap::new();
    let mut prev: Option<LineEvent> = None;
    for line in lines {
        let day = out
            .entry(date_key(line.ts, rollover_hour, tz_offset_secs))
            .or_default();
        day.chars += line.chars;
        if let Some(prev) = prev {
            let gap = line.ts - prev.ts;
            if gap > 0.0 && gap <= session_gap_secs {
                day.active_secs += presence.credit(&prev, gap);
            }
        }
        prev = Some(*line);
    }
    out
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
    use super::super::presence::measure_pace;
    use super::super::testutil::ev;
    use super::*;

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
        let days = aggregate_line_days(
            &lines,
            &Presence::new(&[], measure_pace(&lines, &[], 120.0), 120.0),
            600.0,
            0,
            offset,
        );
        let day1 = days[&NaiveDate::from_ymd_opt(2026, 7, 18).unwrap()];
        let day2 = days[&NaiveDate::from_ymd_opt(2026, 7, 19).unwrap()];
        assert_eq!(day1.chars, 10);
        assert_eq!(day1.active_secs, 0.0);
        assert_eq!(day2.chars, 25);
        // Both gaps sit inside the 120s cap, so both are credited whole — the
        // point of this test is which day they land on, and undisputed gaps are
        // never repriced.
        assert_eq!(day2.active_secs, 120.0);
        assert!(d1 < d2);
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
