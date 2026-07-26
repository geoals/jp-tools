//! Lookups per 1000 characters — the unknown-word rate.
//!
//! The one number that says whether a work sits at the comprehension edge, and
//! the reason it lives alone here is the floor: below a few hundred characters
//! the ratio is dominated by whether you happened to hit one hard name, so it
//! is reported as `None` rather than as a wild number. Two callers compute it
//! (the per-day figure and the per-kind figure in [`super::dialogue`]) and they
//! must agree on that floor.
//!
//! **`chars` is hooked characters only — never a day's total.** A lookup is
//! recorded only while the line stream is live (`routes::ankiproxy::record`),
//! so manually logged reading can enter this denominator but can never enter
//! the numerator: every article logged would push the rate down without any
//! possibility of pushing it up. The rate measures what hooked reading cost,
//! and its denominator has to be the reading it could observe.

/// Characters below which the rate is noise rather than signal.
const MIN_CHARS: i64 = 500;

/// Lookups per 1000 characters, or `None` when there isn't enough text behind
/// it to mean anything.
pub fn lookups_per_1k(lookups: i64, chars: i64) -> Option<f64> {
    (chars >= MIN_CHARS).then(|| lookups as f64 * 1000.0 / chars as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floors_thin_samples() {
        assert_eq!(lookups_per_1k(3, 499), None);
        assert_eq!(lookups_per_1k(3, 500), Some(6.0));
        assert_eq!(lookups_per_1k(0, 10_000), Some(0.0));
    }
}
