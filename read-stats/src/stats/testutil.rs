//! Line fixtures shared by the derivation tests.
//!
//! Timestamps are bare seconds from zero rather than real epoch values: every
//! function under test takes its rollover hour and tz offset as parameters, so
//! a fixture never has to care what day it lands on.

use super::line::LineEvent;

pub(crate) fn ev(ts: f64, chars: i64) -> LineEvent {
    LineEvent { ts, chars }
}
