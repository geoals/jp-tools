//! Line fixtures shared by the derivation tests.
//!
//! Timestamps are bare seconds from zero rather than real epoch values: every
//! function under test takes its rollover hour and tz offset as parameters, so
//! a fixture never has to care what day it lands on.

use super::line::LineEvent;

/// A line with no text to classify — the shape of a row logged before the
/// text column existed, and the default for tests that only care about time.
pub(crate) fn ev(ts: f64, chars: i64) -> LineEvent {
    LineEvent {
        ts,
        chars,
        dialogue_chars: 0,
        classified: false,
    }
}

/// A line of pure dialogue, for the dialogue aggregates.
pub(crate) fn spoken(ts: f64, chars: i64) -> LineEvent {
    LineEvent {
        ts,
        chars,
        dialogue_chars: chars,
        classified: true,
    }
}

/// A line of pure narration.
pub(crate) fn prose(ts: f64, chars: i64) -> LineEvent {
    LineEvent {
        ts,
        chars,
        dialogue_chars: 0,
        classified: true,
    }
}
