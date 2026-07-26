//! Per-work (per-VN) totals.
//!
//! Works join by exact title, the string `vn-ws-logger.py` stamps on each line.
//! An unlabeled line aggregates under `None` rather than being dropped, so the
//! works view always accounts for the whole stream.

use std::collections::BTreeMap;

use serde::Serialize;

/// The one row every logged article aggregates under.
///
/// An article *is* a source, but it is not a work in the sense this view is
/// about — a thing being read through, with a cover, a status and a position
/// in the queue. Thirty of them would bury the four VNs the list exists for,
/// and each would carry a kanji fingerprint built from two thousand
/// characters, which is noise wearing a title. Collapsed, they make one
/// fingerprint worth reading: what your article reading looks like next to
/// your fiction.
///
/// The individual title and URL are not lost — they stay on the session row
/// and show in the day's sittings table, which is where "what did I read on
/// Tuesday" is actually asked.
pub const ARTICLES_WORK: &str = "Articles";

/// The title a manually logged session aggregates under. Articles collapse to
/// [`ARTICLES_WORK`]; a book or a manga keeps its own title, because those are
/// read through over weeks and are exactly what the works view is for.
pub fn work_key(source: &str, work: Option<&str>) -> Option<String> {
    match source {
        "article" => Some(ARTICLES_WORK.to_string()),
        _ => work.map(str::to_string),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
