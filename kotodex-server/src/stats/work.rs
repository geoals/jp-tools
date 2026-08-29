//! Per-work (per-VN) totals.
//!
//! Works join by exact title, the string ingest stamps on each line.
//! An unlabeled line aggregates under `None` rather than being dropped, so the
//! works view always accounts for the whole stream.

use std::collections::BTreeMap;

use serde::Serialize;

use super::line::LineEvent;
use super::presence::Presence;

/// The one row every logged article aggregates under.
///
/// An article *is* a source, but it is not a work in the sense this view is
/// about — a thing being read through, with a cover, a status and a position
/// in the queue. Thirty of them would bury the four VNs the list exists for,
/// and each would be a title over two thousand characters of text, which is
/// noise wearing a name. Collapsed, they make one row worth reading: what your
/// article reading looks like next to your fiction.
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
    /// The line itself, so gap credit can be priced by [`Presence`] exactly as
    /// it is everywhere else.
    pub event: LineEvent,
    pub work: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct WorkAgg {
    pub chars: i64,
    pub active_secs: f64,
    pub first_ts: f64,
    pub last_ts: f64,
}

/// Aggregate a time-ordered line stream per work title. Gap credit goes to the
/// *later* line's work, so a mid-session switch splits time at the switch
/// point.
///
/// Credit is priced by [`Presence`] rather than by a fresh `min(gap, afk)`:
/// the shelf sits one click away from each work's own page, which derives
/// sessions the ordinary way, and two answers to "how many hours is this
/// work" is exactly what the one-rule invariant exists to prevent.
pub fn aggregate_works(
    lines: &[WorkLine],
    presence: &Presence,
    session_gap_secs: f64,
) -> BTreeMap<Option<String>, WorkAgg> {
    let mut out: BTreeMap<Option<String>, WorkAgg> = BTreeMap::new();
    let mut prev: Option<&LineEvent> = None;
    for line in lines {
        let agg = out.entry(line.work.clone()).or_insert_with(|| WorkAgg {
            first_ts: line.event.ts,
            ..Default::default()
        });
        agg.chars += line.event.chars;
        agg.last_ts = line.event.ts;
        if let Some(prev) = prev {
            let gap = line.event.ts - prev.ts;
            if gap > 0.0 && gap <= session_gap_secs {
                agg.active_secs += presence.credit(prev, gap);
            }
        }
        prev = Some(&line.event);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::presence::measure_pace;
    use super::super::testutil::ev;
    use super::*;

    #[test]
    fn works_split_at_switch_point() {
        let w = |ts: f64, chars: i64, work: &str| WorkLine {
            event: ev(ts, chars),
            work: (!work.is_empty()).then(|| work.to_string()),
        };
        let lines = [
            w(0.0, 10, "A"),
            w(30.0, 10, "A"),
            w(60.0, 10, "B"), // switch: this gap credits B
            w(90.0, 10, "B"),
            w(1000.0, 5, ""), // unlabeled, new session (gap > 600)
        ];
        let events: Vec<LineEvent> = lines.iter().map(|l| l.event).collect();
        let presence = Presence::new(&[], measure_pace(&events, &[], 120.0), 120.0);
        let works = aggregate_works(&lines, &presence, 600.0);
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
