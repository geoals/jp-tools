//! SQLite access, one module per table family.
//!
//! Everything here is thin: bind parameters, run a statement, map rows to a
//! struct. No derivation, no policy — that lives in [`crate::stats`] (pure
//! functions over the rows these return) and in [`crate::history`] (which
//! decides *which* rows a request needs). Keeping it thin is what lets the
//! derivations be unit-tested without a database.
//!
//! | module | tables | owner |
//! |---|---|---|
//! | [`pool`] | — | connection + migrations |
//! | [`settings`] | `settings` | read-stats |
//! | [`pauses`] | `pauses` | read-stats |
//! | [`marks`] | `reader_marks` | read-stats |
//! | [`covers`] | `work_covers` | read-stats |
//! | [`lines`] | `lines` | knowledge (shared) |
//! | [`works`] | `works` | knowledge (shared) |
//! | [`sessions`] | `sessions` | knowledge (shared) |
//! | [`anki_notes`] | `anki_notes` | knowledge (shared) |
//! | [`word_days`] | `word_days` | knowledge (shared) |
//! | [`lookups`] | `lookups` | knowledge (shared) |
//!
//! The "owner" column is where `spec/knowledge-db.md` places each table: the
//! shared ones are dictionary-gated knowledge that other tools will read, and
//! are destined to move into `jp-core`'s `knowledge.db`. They all live in one
//! file today; the split above is drawn where that seam will fall, and no query
//! in this crate joins across it.

pub mod anki_notes;
pub mod covers;
pub mod lines;
pub mod lookups;
pub mod marks;
pub mod pauses;
pub mod pool;
pub mod sessions;
pub mod settings;
pub mod word_days;
pub mod works;

pub use anki_notes::{AnkiNote, fetch_anki_note_ids, fetch_anki_notes, replace_anki_notes};
pub use covers::{clear_work_cover_vndb, fetch_work_covers, set_work_cover_vndb};
pub use lines::{
    ClassifiedLine, IngestLine, ReaderLine, fetch_classified_lines, fetch_line_events,
    fetch_lines_after, fetch_lines_after_id, fetch_recent_lines, fetch_work_lines, max_line_id,
    set_lines_discarded,
};
pub use lookups::{LookupTerm, fetch_lookup_events, fetch_lookup_terms, insert_lookup};
pub use marks::{fetch_reader_marks, insert_reader_mark};
pub use pauses::{fetch_pauses, is_pause_open, toggle_pause};
pub use pool::create_pool;
pub use sessions::{ManualSession, delete_session, fetch_sessions, insert_session};
pub use settings::{SETTING_KEYS, Settings, get_setting_raw, load_settings, save_setting};
pub use word_days::{WordDayHit, add_word_day_counts, fetch_mined_word_days};
pub use works::{
    WORK_STATUSES, Work, current_work_vn_window, delete_work, fetch_work, fetch_works_meta,
    set_work_cover, set_work_queue_pos, set_work_status, set_work_total_chars, set_work_vn_window,
    upsert_work,
};
