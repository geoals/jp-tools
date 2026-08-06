//! The vocabulary count as a curve: new words per day, and the running total.
//!
//! One word is dated once, by the day it was first called known. The bars are
//! that day's new words, the line is their sum, and the last point is the same
//! number the vocabulary tile shows.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct GrowthDay {
    pub date: String,
    pub new: usize,
    pub cumulative: usize,
}

/// Bucket one date per word into a daily series.
///
/// Every day between the first and the last is emitted, including the empty
/// ones: a flat stretch is a fact about the reading, and dropping it would draw
/// a week's pause as a single steep step.
pub fn growth_days(first_known: &[NaiveDate], today: NaiveDate) -> Vec<GrowthDay> {
    let mut per_day: BTreeMap<NaiveDate, usize> = BTreeMap::new();
    for date in first_known {
        *per_day.entry(*date).or_default() += 1;
    }
    let Some((&start, _)) = per_day.iter().next() else {
        return Vec::new();
    };
    let end = today.max(*per_day.keys().next_back().unwrap());

    let mut cumulative = 0;
    let mut out = Vec::new();
    let mut date = start;
    while date <= end {
        let new = per_day.get(&date).copied().unwrap_or(0);
        cumulative += new;
        out.push(GrowthDay {
            date: date.to_string(),
            new,
            cumulative,
        });
        date += chrono::Duration::days(1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn counts_per_day_and_accumulates() {
        let days = growth_days(
            &[d("2026-01-01"), d("2026-01-01"), d("2026-01-03")],
            d("2026-01-03"),
        );
        let seen: Vec<_> = days.iter().map(|g| (&g.date, g.new, g.cumulative)).collect();
        assert_eq!(
            seen,
            vec![
                (&"2026-01-01".to_string(), 2, 2),
                (&"2026-01-02".to_string(), 0, 2),
                (&"2026-01-03".to_string(), 1, 3),
            ]
        );
    }

    #[test]
    fn runs_to_today_so_a_pause_is_visible() {
        let days = growth_days(&[d("2026-01-01")], d("2026-01-04"));
        assert_eq!(days.len(), 4);
        assert_eq!(days.last().unwrap().cumulative, 1);
    }

    #[test]
    fn empty_ledger_is_no_series() {
        assert!(growth_days(&[], d("2026-01-01")).is_empty());
    }
}
