//! Response fragments shared by more than one endpoint.
//!
//! Kept together so that a field added to (say) the focus block appears on
//! every endpoint that reports focus, rather than on whichever one was edited.

use serde_json::{Value, json};

use crate::stats::{FocusDay, Side};

pub fn focus_json(f: &FocusDay) -> Value {
    json!({
        "ratio": f.ratio(),
        "span_secs": f.span_secs,
        "longest_stretch_secs": f.longest_stretch_secs,
        "interruptions": f.interruptions,
    })
}

pub fn side_json(s: &Side) -> Value {
    json!({
        "chars": s.chars,
        "timed_chars": s.timed_chars,
        "timed_secs": s.timed_secs,
        "lines": s.lines,
        "speed": s.speed(),
        "clean_speed": s.clean_speed(),
        "lookups": s.lookups,
        "lookups_per_1k": s.lookups_per_1k(),
    })
}
