//! The reading history, loaded once per request: the line stream, the evidence
//! the reader was present, and the settings that price it.
//!
//! The point is not only the one query. **Pace and presence are properties of
//! the reader, not of a request**: an endpoint that derives its own measures
//! pace over whatever window it asked about, and the same day then reports
//! different active minutes depending on the page. One place decides, and
//! endpoints can only ask it questions.
//!
//! Windowed endpoints slice these vectors rather than re-querying, and pad the
//! window so a session straddling the rollover derives against its real
//! neighbours.

use std::collections::BTreeMap;

use chrono::NaiveDate;

use crate::app::AppState;
use crate::clock::{now_ts, tz_offset_secs};
use crate::db::{self, ManualSession, Settings};
use crate::error::AppError;
use crate::stats::{self, DayBucket, FocusDay, LineEvent, Presence, WorkLine, date_key};

/// A work must account for at least this fraction of a day's characters to
/// count as that day's reading rather than a passing glance at another VN.
const DOMINANT_SHARE_FLOOR: f64 = 0.2;

pub struct History {
    pub settings: Settings,
    /// Fixed for the life of the request, so every date in one response is
    /// keyed against the same boundary.
    pub tz: i64,
    pub today: NaiveDate,
    /// The whole line stream, oldest first. Discarded lines are filtered out in
    /// SQL and nothing else is excluded — capture stops at the source, so a line
    /// that exists is a line that counts.
    pub lines: Vec<LineEvent>,
    /// Work title per entry of [`Self::lines`] — same order, same length. Kept
    /// alongside rather than inside `LineEvent` so the derivations stay `Copy`.
    pub line_works: Vec<Option<String>>,
    /// Yomitan lookup timestamps, sorted.
    pub lookups: Vec<f64>,
    /// Everything proving the reader was at the keyboard — lookups, mined
    /// cards, and deliberate #read actions — merged and sorted.
    pub marks: Vec<f64>,
    /// Mined note ids, ascending. These are epoch *milliseconds*, so they
    /// double as card creation times.
    pub note_ids: Vec<i64>,
    pub manual: Vec<ManualSession>,
    /// Chars per second over undisputed gaps, across all history.
    pub pace: Option<f64>,
    /// Chars per second over recent reading, interruptions included — what an
    /// untimed session's duration is derived from. See [`Self::duration_of`].
    pub effective_pace: Option<f64>,
}

/// How far back [`History::effective_pace`] looks — recent enough to be the
/// speed the untimed reading happened at, long enough to average over hard days
/// and easy ones.
const EFFECTIVE_PACE_DAYS: f64 = 30.0;

impl History {
    pub async fn load(state: &AppState) -> Result<Self, AppError> {
        let settings = db::load_settings(&state.local).await?;
        let tz = tz_offset_secs();
        let today = date_key(now_ts(), settings.day_rollover_hour, tz);

        let classified = db::fetch_worked_lines(&state.knowledge, 0.0, f64::MAX).await?;
        let mut lines = Vec::with_capacity(classified.len());
        let mut line_works = Vec::with_capacity(classified.len());
        for c in classified {
            lines.push(c.event);
            line_works.push(c.work);
        }

        let lookups = db::fetch_lookup_events(&state.knowledge, 0.0, f64::MAX).await?;

        let note_ids = db::fetch_anki_note_ids(&state.knowledge).await?;
        let cards: Vec<f64> = note_ids.iter().map(|id| *id as f64 / 1000.0).collect();
        let reader = db::fetch_reader_marks(&state.local, 0.0, f64::MAX).await?;
        let marks = stats::presence_marks(&lookups, &cards, &reader);

        let manual = db::fetch_sessions(&state.knowledge, 0.0, f64::MAX).await?;

        let pace = stats::measure_pace(&lines, &marks, settings.afk_secs);
        // Needs the presence rule, which needs the pace — so it is built here
        // rather than in the struct literal below.
        let effective_pace = stats::measure_effective_pace(
            &lines,
            &Presence::new(&marks, pace, settings.afk_secs),
            settings.session_gap_secs,
            now_ts() - EFFECTIVE_PACE_DAYS * 86400.0,
        );

        Ok(History {
            settings,
            tz,
            today,
            lines,
            line_works,
            lookups,
            marks,
            note_ids,
            manual,
            pace,
            effective_pace,
        })
    }

    /// How long a manually logged session took, and whether that is measured or
    /// estimated.
    ///
    /// A book is read without a stopwatch, so when `end_ts` is absent the time
    /// is derived from the characters at the reader's own recent effective pace.
    /// The estimate improves as the measured pace does.
    ///
    /// Zero when there is no pace to divide by yet, deliberately rather than a
    /// guess — any duration here would land in the goal meter and the streak.
    pub fn duration_of(&self, s: &db::ManualSession) -> (f64, bool) {
        match s.end_ts {
            Some(end) => ((end - s.start_ts).max(0.0), false),
            None => match self.effective_pace {
                Some(p) if p > 0.0 => (s.chars as f64 / p, true),
                _ => (0.0, true),
            },
        }
    }

    /// The one presence rule, with the one pace. Everything that credits gap
    /// time goes through this.
    pub fn presence(&self) -> Presence<'_> {
        Presence::new(&self.marks, self.pace, self.settings.afk_secs)
    }

    /// Lines in `[from, to)`. The slice is sorted by timestamp, so this is two
    /// binary searches.
    pub fn lines_in(&self, from: f64, to: f64) -> &[LineEvent] {
        let lo = self.lines.partition_point(|l| l.ts < from);
        let hi = self.lines.partition_point(|l| l.ts < to);
        &self.lines[lo..hi]
    }

    pub fn lookups_in(&self, from: f64, to: f64) -> &[f64] {
        slice_range(&self.lookups, from, to)
    }

    /// Mined card creation times in `[from, to)`, in seconds.
    pub fn card_times_in(&self, from: f64, to: f64) -> Vec<f64> {
        let (a, b) = ((from * 1000.0) as i64, (to * 1000.0) as i64);
        let lo = self.note_ids.partition_point(|&id| id < a);
        let hi = self.note_ids.partition_point(|&id| id < b);
        self.note_ids[lo..hi]
            .iter()
            .map(|id| *id as f64 / 1000.0)
            .collect()
    }

    /// How many cards were mined in `[from, to)`.
    pub fn cards_in(&self, from: f64, to: f64) -> i64 {
        let (a, b) = ((from * 1000.0) as i64, (to * 1000.0) as i64);
        let lo = self.note_ids.partition_point(|&id| id < a);
        let hi = self.note_ids.partition_point(|&id| id < b);
        (hi - lo) as i64
    }

    pub fn day_start(&self, date: NaiveDate) -> f64 {
        stats::day_start_ts(date, self.settings.day_rollover_hour, self.tz)
    }

    pub fn date_of(&self, ts: f64) -> NaiveDate {
        date_key(ts, self.settings.day_rollover_hour, self.tz)
    }

    /// Per-day totals from the line stream, and from manually logged sessions,
    /// kept apart so the dashboard can show what came from where.
    pub fn day_maps(
        &self,
    ) -> (
        BTreeMap<NaiveDate, DayBucket>,
        BTreeMap<NaiveDate, DayBucket>,
    ) {
        let vn = stats::aggregate_line_days(
            &self.lines,
            &self.presence(),
            self.settings.session_gap_secs,
            self.settings.day_rollover_hour,
            self.tz,
        );

        let mut manual: BTreeMap<NaiveDate, DayBucket> = BTreeMap::new();
        for s in &self.manual {
            let day = manual.entry(self.date_of(s.start_ts)).or_default();
            day.chars += s.chars;
            day.active_secs += self.duration_of(s).0;
        }
        (vn, manual)
    }

    /// Per-day totals over reading whose duration was actually *measured* — the
    /// line stream, plus manual sessions logged with minutes.
    ///
    /// The denominator every chars/hour figure divides by. It exists because of
    /// circularity: an estimated session's duration comes from the reader's own
    /// pace, so feeding it to a speed chart makes the chart measure its own
    /// output. Totals, goals and streaks deliberately do *not* use this —
    /// speed is the only question an estimate cannot answer honestly.
    pub fn measured_days(&self) -> BTreeMap<NaiveDate, DayBucket> {
        let mut out = stats::aggregate_line_days(
            &self.lines,
            &self.presence(),
            self.settings.session_gap_secs,
            self.settings.day_rollover_hour,
            self.tz,
        );
        for s in &self.manual {
            let (secs, estimated) = self.duration_of(s);
            if estimated {
                continue;
            }
            let day = out.entry(self.date_of(s.start_ts)).or_default();
            day.chars += s.chars;
            day.active_secs += secs;
        }
        out
    }

    /// Per-day characters and time with the lookups taken out: characters whose
    /// own gap held no lookup, over what those gaps cost. Both sides drop
    /// together — see [`stats::Bucket::clean_chars`]. The bucket width is
    /// arbitrary here since this only re-aggregates to whole days.
    pub fn clean_days(&self) -> BTreeMap<NaiveDate, DayBucket> {
        let mut out: BTreeMap<NaiveDate, DayBucket> = BTreeMap::new();
        for b in stats::bucket_lines(
            &self.lines,
            &self.lookups,
            &self.presence(),
            self.settings.session_gap_secs,
            60.0,
        ) {
            let day = out.entry(self.date_of(b.t)).or_default();
            day.chars += b.clean_chars;
            day.active_secs += b.active_secs - b.lookup_secs;
        }
        out
    }

    /// The two day maps added together — what the goal meter and the streak
    /// count against.
    pub fn total_days(&self) -> BTreeMap<NaiveDate, DayBucket> {
        let (vn, manual) = self.day_maps();
        let mut out = vn;
        for (date, bucket) in manual {
            let day = out.entry(date).or_default();
            day.chars += bucket.chars;
            day.active_secs += bucket.active_secs;
        }
        out
    }

    /// Per-day focus (how continuous the reading was) from the raw line stream.
    pub fn focus_days(&self) -> BTreeMap<NaiveDate, FocusDay> {
        stats::aggregate_focus_days(
            &self.lines,
            &self.presence(),
            self.settings.session_gap_secs,
            self.settings.day_rollover_hour,
            self.tz,
        )
    }

    /// Yomitan lookups per day.
    pub fn lookup_days(&self) -> BTreeMap<NaiveDate, i64> {
        let mut out: BTreeMap<NaiveDate, i64> = BTreeMap::new();
        for &ts in &self.lookups {
            *out.entry(self.date_of(ts)).or_default() += 1;
        }
        out
    }

    /// The line stream paired with its work titles, for the per-work totals.
    pub fn work_lines(&self) -> Vec<WorkLine> {
        self.lines
            .iter()
            .zip(&self.line_works)
            .map(|(l, work)| WorkLine {
                event: *l,
                work: work.clone(),
            })
            .collect()
    }

    /// Only the lines stamped with `work`, for scoping a summary to one VN.
    ///
    /// Interleaving is session-level or coarser in practice, so a gap bridging
    /// two of this work's lines across an interlude in another exceeds
    /// `session_gap_secs` and is dropped. Line-by-line alternation inside one
    /// sitting would leak, but nobody reads two VNs that way.
    pub fn lines_of_work(&self, work: &str) -> Vec<LineEvent> {
        self.lines
            .iter()
            .zip(&self.line_works)
            .filter(|(_, w)| w.as_deref() == Some(work))
            .map(|(l, _)| *l)
            .collect()
    }

    /// Which work was being read at `ts`, if any.
    ///
    /// The same test `ankiproxy::record` applies at the write: an event belongs
    /// to the reading only if a line arrived within `session_gap_secs`. The
    /// nearest line in either direction names the work, since a card added just
    /// after a sitting's last line is still that sitting's.
    ///
    /// `None` when nothing was on screen, rather than a guess — that keeps a
    /// work from claiming mining that happened somewhere else.
    pub fn work_at(&self, ts: f64) -> Option<&str> {
        let gap = self.settings.session_gap_secs;
        let i = self.lines.partition_point(|l| l.ts < ts);
        let nearest = [i.checked_sub(1), (i < self.lines.len()).then_some(i)]
            .into_iter()
            .flatten()
            .min_by(|&a, &b| {
                (self.lines[a].ts - ts)
                    .abs()
                    .total_cmp(&(self.lines[b].ts - ts).abs())
            })?;
        ((self.lines[nearest].ts - ts).abs() <= gap)
            .then(|| self.line_works[nearest].as_deref())
            .flatten()
    }

    /// The work that best represents each day's reading, so a speed step on the
    /// trend charts reads as a switch rather than a regression.
    ///
    /// The *latest* work read that day that cleared a real share of its
    /// characters: latest rather than heaviest, so a mid-day switch onto a new
    /// VN still shows; the share floor stops a brief end-of-day peek at another
    /// VN flipping the day.
    ///
    /// Hooked lines only. A logged book or article has no measured duration, so
    /// it is not in the speed series the marker annotates.
    pub fn dominant_work_days(&self) -> BTreeMap<NaiveDate, String> {
        // Per day, each work's total chars and the latest timestamp it was read at.
        let mut per_day: BTreeMap<NaiveDate, BTreeMap<String, (i64, f64)>> = BTreeMap::new();
        let mut tally = |work: Option<&str>, ts: f64, chars: i64| {
            if let Some(work) = work.filter(|w| !w.is_empty()) {
                let e = per_day
                    .entry(date_key(ts, self.settings.day_rollover_hour, self.tz))
                    .or_default()
                    .entry(work.to_string())
                    .or_default();
                e.0 += chars;
                e.1 = e.1.max(ts);
            }
        };
        for (line, work) in self.lines.iter().zip(&self.line_works) {
            tally(work.as_deref(), line.ts, line.chars);
        }

        per_day
            .into_iter()
            .filter_map(|(day, works)| {
                let total: i64 = works.values().map(|(chars, _)| *chars).sum();
                let floor = total as f64 * DOMINANT_SHARE_FLOOR;
                works
                    .iter()
                    .filter(|(_, (chars, _))| *chars as f64 >= floor)
                    .max_by(|(_, (_, a)), (_, (_, b))| a.total_cmp(b))
                    // Fall back to the day's heaviest work if a busy day split
                    // every work below the floor.
                    .or_else(|| works.iter().max_by_key(|(_, (chars, _))| *chars))
                    .map(|(work, _)| (day, work.clone()))
            })
            .collect()
    }
}

/// The subslice of a sorted timestamp list covering `[from, to)`.
fn slice_range(sorted: &[f64], from: f64, to: f64) -> &[f64] {
    let lo = sorted.partition_point(|&ts| ts < from);
    let hi = sorted.partition_point(|&ts| ts < to);
    &sorted[lo..hi]
}
