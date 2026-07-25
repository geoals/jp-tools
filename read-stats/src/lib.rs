//! Daily reading tracker: characters and active time derived from the raw line
//! stream a texthooker writes, plus the phone-side reading view.
//!
//! The layers, outermost first:
//!
//! - [`app`] / [`routes`] — HTTP. Thin: parse, ask, shape JSON.
//! - [`history`] — the reading history loaded once per request, and the single
//!   place that decides pace and presence.
//! - [`stats`] — pure derivation. No database, no clock, no timezone; every
//!   threshold arrives as a parameter. This is where the metrics are *defined*.
//! - [`db`] — SQLite, one module per table family. Thin the other way: bind,
//!   run, map rows.
//! - [`clock`] — the only two impure inputs ([`clock::now_ts`],
//!   [`clock::tz_offset_secs`]), so their reach is greppable.
//!
//! Side channels that don't fit the stack: [`anki`] (AnkiConnect client),
//! [`ankiproxy`] (the endpoint Yomitan points at, which is how lookups become
//! observable), [`llm`] + [`compactdef`] + [`tags`] (card enrichment),
//! [`covers`] + [`vndb`] (work art), [`ingest`] (tokenizing new lines).
//!
//! Character counting and the speech/prose split are **not** here — they are
//! `jp_core::text`, shared with the other tools, because a character counted by
//! this crate's speed figures has to be the same character everywhere else.

pub mod anki;
pub mod ankiproxy;
pub mod app;
pub mod clock;
pub mod compactdef;
pub mod config;
pub mod covers;
pub mod db;
pub mod error;
pub mod history;
pub mod ingest;
pub mod llm;
pub mod routes;
pub mod stats;
pub mod tags;
pub mod vndb;
