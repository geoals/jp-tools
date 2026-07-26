//! SQLite access, one module per table family.
//!
//! Everything here is thin: bind parameters, run a statement, map rows to a
//! struct. No derivation, no policy — that lives in [`crate::stats`] (pure
//! functions over the rows these return) and in [`crate::history`] (which
//! decides *which* rows a request needs). Keeping it thin is what lets the
//! derivations be unit-tested without a database.
//!
//! There are **two databases**, and which one a module talks to is visible in
//! its signature: knowledge modules take a [`jp_core::knowledge::Knowledge`],
//! local ones take a bare `SqlitePool`. Both are SQLite pools and both create
//! their file on demand, so passing the wrong one would fail silently against a
//! freshly created empty table — the newtype makes that a compile error.
//!
//! | module | tables | database |
//! |---|---|---|
//! | [`pool`] | — | opens both |
//! | [`settings`] | `settings` | read-stats.db |
//! | [`marks`] | `reader_marks` | read-stats.db |
//! | [`covers`] | `work_covers` | read-stats.db |
//! | [`lines`] | `lines` | knowledge.db |
//! | [`works`] | `works` | knowledge.db |
//! | [`sessions`] | `manual_sessions` | knowledge.db |
//! | [`anki_notes`] | `anki_notes` | knowledge.db |
//! | [`word_days`] | `word_days` | knowledge.db |
//! | [`lookups`] | `lookups` | knowledge.db |
//!
//! The split follows `spec/knowledge-db.md`: what is *about the reading* is
//! shared, because other tools ask questions of it; what is about this app's
//! own behaviour stays local. Only two places straddle the line —
//! [`works::current_work_vn_window`] and [`covers::fetch_work_covers`] — and
//! both do the join in memory rather than attaching one database to the other.

pub mod anki_notes;
pub mod covers;
pub mod lines;
pub mod lookups;
pub mod marks;
pub mod pool;
pub mod retire_pauses;
pub mod sessions;
pub mod settings;
pub mod word_days;
pub mod works;

pub use anki_notes::{AnkiNote, fetch_anki_note_ids, fetch_anki_notes, replace_anki_notes};
pub use covers::{clear_work_cover_vndb, fetch_work_covers, set_work_cover_vndb};
pub use lines::{
    ClassifiedLine, IngestLine, ReaderLine, fetch_classified_lines, fetch_kanji_lines,
    fetch_line_events, fetch_lines_after, fetch_lines_after_id, fetch_recent_lines,
    fetch_work_lines, line_within, max_line_id, set_lines_discarded,
};
pub use lookups::{LookupTerm, fetch_lookup_events, fetch_lookup_terms, insert_lookup};
pub use marks::{fetch_reader_marks, insert_reader_mark};
pub use pool::{create_pool, open_knowledge};
pub use retire_pauses::retire as retire_pauses;
pub use sessions::{ManualSession, delete_session, fetch_sessions, insert_session};
pub use settings::{
    BOOL_SETTING_KEYS, SETTING_KEYS, Settings, get_setting_raw, load_settings, save_setting,
};
pub use word_days::{WordDayHit, add_word_day_counts, fetch_mined_word_days, fetch_word_totals};
pub use works::{
    WORK_STATUSES, Work, current_work_vn_window, delete_work, fetch_work, fetch_works_meta,
    set_work_cover, set_work_queue_pos, set_work_status, set_work_total_chars, set_work_vn_window,
    upsert_work,
};
