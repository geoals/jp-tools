//! The reading history, loaded once per request.
//!
//! Every dashboard endpoint needs the same three things — the line stream, the
//! evidence that the reader was present, and the settings that price it — and
//! each of them used to fetch that itself.
//! `/api/summary` alone ran the full-history line query three times and derived
//! the reading pace twice, because `day_maps`, `focus_days` and `lookup_days`
//! were written independently and each opened with the same six lines of setup.
//!
//! Loading it once is the smaller half of the point. The larger half is that
//! **pace and presence are properties of the reader, not of a request**: while
//! each endpoint derived its own, the dashboard measured pace over all history
//! and the day timeline over one day, so the same day reported different active
//! minutes depending on which page you were looking at. There is now one place
//! that decides, and endpoints can only ask it questions.
//!
//! Windowed endpoints (a day's timeline, a day's sessions) slice these vectors
//! rather than re-querying, and they keep the padded windows they always used
//! — a session straddling the rollover still derives against its real
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
    /// The whole line stream, oldest first. Discarded lines are already gone —
    /// filtered in SQL — and nothing else is excluded: capture stops at the
    /// source now, so a line that exists is a line that counts.
    pub lines: Vec<LineEvent>,
    /// Work title for each entry of [`Self::lines`] — same order, same length.
    /// Kept alongside rather than inside `LineEvent` so the derivations stay
    /// `Copy` and never carry a string they don't look at.
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

/// How far back [`History::effective_pace`] looks. Recent enough to be the
/// speed the untimed reading actually happened at, long enough to average over
/// hard days and easy ones.
const EFFECTIVE_PACE_DAYS: f64 = 30.0;

impl History {
    pub async fn load(state: &AppState) -> Result<Self, AppError> {
        let settings = db::load_settings(&state.local).await?;
        let tz = tz_offset_secs();
        let today = date_key(now_ts(), settings.day_rollover_hour, tz);

        let classified = db::fetch_classified_lines(&state.knowledge, 0.0, f64::MAX).await?;
        let mut lines = Vec::with_capacity(classified.len());
        let mut line_works = Vec::with_capacity(classified.len());
        for c in classified {
            lines.push(c.event);
            line_works.push(c.work);
        }

        let lookups = db::fetch_lookup_events(&state.knowledge, 0.0, f64::MAX).await?;

        let note_ids = db::fetch_anki_note_ids(&state.knowledge).await?;
        // Note ids are epoch milliseconds, so they double as card creation times.
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

    /// How long a manually logged session took, and whether that is a measured
    /// number or an estimate.
    ///
    /// A logged session may carry no duration at all: a book or an article is
    /// read without a stopwatch, and requiring a minute count only produces
    /// invented ones. When `end_ts` is absent the time is derived from the
    /// characters at the reader's own recent effective pace — the same "derive
    /// it, don't store it" rule the rest of the dashboard follows, so the
    /// estimate improves as the measured pace does.
    ///
    /// Zero when there is no pace to divide by yet. That is deliberately not a
    /// guess: with no measured reading behind it, any duration here would be a
    /// number nobody could account for, and it would land in the goal meter and
    /// the streak.
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

    /// Lines in `[from, to)`. The slice is already sorted by timestamp, so this
    /// is two binary searches.
    ///
    /// Note that the speech/prose classification on these events came from
    /// scanning the *whole* stream (bracket depth carries across lines), not
    /// from a scan starting at `from` — which is the more correct of the two,
    /// and in any case windowed callers only read `ts` and `chars`.
    pub fn lines_in(&self, from: f64, to: f64) -> &[LineEvent] {
        let lo = self.lines.partition_point(|l| l.ts < from);
        let hi = self.lines.partition_point(|l| l.ts < to);
        &self.lines[lo..hi]
    }

    pub fn lookups_in(&self, from: f64, to: f64) -> &[f64] {
        slice_range(&self.lookups, from, to)
    }

    pub fn marks_in(&self, from: f64, to: f64) -> &[f64] {
        slice_range(&self.marks, from, to)
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

    /// Per-day totals over reading whose duration was actually *measured* —
    /// the line stream, plus manual sessions that were logged with minutes.
    ///
    /// This is the denominator every chars/hour figure divides by, and the
    /// reason it exists is circularity: an estimated session's duration is
    /// derived from the reader's own pace, so it reports that pace back
    /// exactly. Feeding it into a speed chart would make the chart partly
    /// measure its own output — every untimed article logged would pull the
    /// curve toward the mean and flatten the real variation it exists to show.
    ///
    /// Totals, goals and streaks deliberately do *not* use this: an estimated
    /// session's time is the best available answer to "how long was spent
    /// reading", and it is only speed it cannot honestly speak to.
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
                ts: l.ts,
                chars: l.chars,
                work: work.clone(),
            })
            .collect()
    }

    /// Only the lines stamped with `work`, for scoping a summary to one VN.
    ///
    /// Realistic interleaving is session-level or coarser, so the gaps that
    /// bridge two of this work's lines across an interlude in another exceed
    /// `session_gap_secs` and are dropped — the same per-work handling the rest
    /// of the app relies on. (Line-by-line alternation inside one sitting would
    /// leak, but nobody reads two VNs that way.)
    pub fn lines_of_work(&self, work: &str) -> Vec<LineEvent> {
        self.lines
            .iter()
            .zip(&self.line_works)
            .filter(|(_, w)| w.as_deref() == Some(work))
            .map(|(l, _)| *l)
            .collect()
    }

    /// The work that best represents each day's reading, so the trend charts can
    /// mark where reading moved from one VN to another and a speed step reads as
    /// a switch rather than a regression. Picks the *latest* work read that day
    /// that cleared a real share of the day's characters — latest-not-dominant
    /// so a mid-day switch onto a new VN still shows even while a heavier
    /// earlier work out-charactered it, and the share floor keeps a brief
    /// end-of-day peek at another VN from flipping the day. Manual sessions join
    /// in too — a day of physical-book reading names the book.
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
        for s in &self.manual {
            // Articles name the day as "Articles" rather than as one headline.
            tally(
                stats::work_key(&s.source, s.work.as_deref()).as_deref(),
                s.start_ts,
                s.chars,
            );
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
