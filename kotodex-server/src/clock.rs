//! The two impure inputs every derivation takes as a parameter.
//!
//! [`crate::stats`] is deterministic because it never asks what time it is or
//! what timezone it's in — those come in as arguments. This module is where
//! they enter the program, so "does this depend on the wall clock?" is answered
//! by grepping for these two names.

use chrono::{Local, NaiveDate, TimeZone};

/// Seconds to add to UTC to get local time, at this instant.
///
/// Read fresh rather than cached: it changes twice a year, and a long-running
/// server that cached it in March would report April's days shifted by an hour.
pub fn tz_offset_secs() -> i64 {
    Local::now().offset().local_minus_utc() as i64
}

/// Now, in epoch seconds — the same unit `lines.ts` is stored in.
pub fn now_ts() -> f64 {
    if let Some(pinned) = pinned_now() {
        return pinned;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// `KOTODEX_DEMO_TODAY=YYYY-MM-DD`, as an instant late on that day.
///
/// The public demo serves a frozen history, so a real clock walks it off the
/// end: every day that passes is another empty one, and within a week Today is
/// blank and the streak reads zero. Pinning the clock to a day that has reading
/// in it keeps the dashboard showing a dashboard.
///
/// Late in the day rather than midnight, so the day's own lines are in the past
/// and Today is complete rather than half-filled.
fn pinned_now() -> Option<f64> {
    static PINNED: std::sync::OnceLock<Option<f64>> = std::sync::OnceLock::new();
    *PINNED.get_or_init(|| {
        let raw = std::env::var("KOTODEX_DEMO_TODAY").ok()?;
        let date = raw.trim().parse::<NaiveDate>().ok()?;
        let at = date.and_hms_opt(23, 30, 0)?;
        Some(Local.from_local_datetime(&at).single()?.timestamp() as f64)
    })
}
