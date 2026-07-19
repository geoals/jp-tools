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
