//! Response fragments shared by more than one endpoint.
//!
//! Kept together so that a field added to (say) the focus block appears on
//! every endpoint that reports focus, rather than on whichever one was edited.

use serde_json::{Value, json};

use crate::stats::FocusDay;

pub fn focus_json(f: &FocusDay) -> Value {
    json!({
        "ratio": f.ratio(),
        "span_secs": f.span_secs,
        "longest_stretch_secs": f.longest_stretch_secs,
        "interruptions": f.interruptions,
    })
}
