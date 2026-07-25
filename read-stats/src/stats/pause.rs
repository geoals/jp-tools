//! Which lines count at all.
//!
//! A pause is the prospective half of "don't count this" — the button pressed
//! before re-reading a stretch or wandering through a route-select screen. (The
//! retroactive half is the `discarded` flag on the line itself, applied by the
//! reader view after the fact.) Nothing is ever deleted either way: the raw
//! rows stay, and every derivation filters.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paused_intervals_cover_lines() {
        let pauses = [
            PauseInterval {
                start_ts: 100.0,
                end_ts: Some(200.0),
            },
            PauseInterval {
                start_ts: 500.0,
                end_ts: None,
            },
        ];
        assert!(!is_paused(50.0, &pauses));
        assert!(is_paused(100.0, &pauses));
        assert!(is_paused(150.0, &pauses));
        assert!(!is_paused(200.0, &pauses)); // end is exclusive
        assert!(!is_paused(300.0, &pauses));
        assert!(is_paused(9999.0, &pauses)); // open interval
    }
}
