//! The two impure inputs every derivation takes as a parameter.
//!
//! [`crate::stats`] is deterministic because it never asks what time it is or
//! what timezone it's in — those come in as arguments. This module is where
//! they enter the program, so "does this depend on the wall clock?" is answered
//! by grepping for these two names.

use chrono::Local;

/// Seconds to add to UTC to get local time, at this instant.
///
/// Read fresh rather than cached: it changes twice a year, and a long-running
/// server that cached it in March would report April's days shifted by an hour.
pub fn tz_offset_secs() -> i64 {
    Local::now().offset().local_minus_utc() as i64
}

/// Now, in epoch seconds — the same unit `lines.ts` is stored in.
pub fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
