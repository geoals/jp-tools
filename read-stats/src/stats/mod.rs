//! Pure derivation logic: raw line events → sessions, active time, per-day
//! aggregates. All functions take thresholds and a fixed tz offset as
//! parameters so they stay deterministic and unit-testable — nothing in here
//! touches the database, the clock, or the local timezone.
//!
//! Everything downstream of the line stream is derived, never stored, so this
//! is where the reading metrics are actually defined. The modules, in the order
//! the data flows through them:
//!
//! | module | answers |
//! |---|---|
//! | [`line`] | the one input type — a hooked line reduced to what derivation needs |
//! | [`pause`] | which lines count at all |
//! | [`presence`] | how much of each inter-line gap was really reading (the core rule) |
//! | [`session`] | where one sitting ends and the next begins |
//! | [`day`] | per-day chars/time, the calendar boundary, streaks |
//! | [`timeline`] | one day sliced into fine buckets — the intra-day speed curve |
//! | [`work`] | per-VN totals |
//! | [`focus`] | how *continuous* the reading was, as opposed to how much |
//! | [`dialogue`] | how much of it was people talking, and at what speed |
//!
//! [`presence`] is the module to read first: every other aggregate credits gap
//! time through it, and keeping that rule in one place is what stops two views
//! of the same day disagreeing.

pub mod dialogue;
pub mod focus;
pub mod line;
pub mod pause;
pub mod presence;
pub mod session;
pub mod timeline;
pub mod work;

pub mod day;

#[cfg(test)]
pub(crate) mod testutil;

pub use day::{DayBucket, aggregate_line_days, date_key, day_start_ts, streaks};
pub use dialogue::{DialogueDay, Side, aggregate_dialogue_days};
pub use focus::{FocusDay, INTERRUPTION_SECS, aggregate_focus_days};
pub use line::LineEvent;
pub use pause::{PauseInterval, is_paused};
pub use presence::{Presence, measure_pace, presence_marks};
pub use session::{Session, derive_sessions};
pub use timeline::{Bucket, EventKind, add_events, bucket_lines};
pub use work::{WorkAgg, WorkLine, aggregate_works};
