//! Talking to things that aren't this program.
//!
//! Everything here crosses a process or network boundary, which is what
//! separates it from [`crate::stats`] (pure) and [`crate::db`] (local storage).
//! They share one rule: a side channel failing must never take the request with
//! it. A cover that won't download, a whisper-service that isn't up, an
//! enrichment call that times out — each degrades to a missing field, not a 500.
//!
//! | module | talks to |
//! |---|---|
//! | [`anki`] | AnkiConnect, read-only — snapshots the mined deck |
//! | [`audio`] | the Local Audio Server for Yomitan, for a word's pronunciation |
//! | [`card`] | AnkiConnect — adds a card, then enriches it; every card path's seam |
//! | [`capture`] | vn-mine's `vn-capture.sh`, and `xdotool` for window titles |
//! | [`notify`] | `notify-send` — the one report a finished mine makes |
//! | [`llm`] | the Anthropic API |
//! | [`compactdef`] | — (builds the prompt [`llm`] sends, then the field value) |
//! | [`tags`] | — (the two-axis tag rubric both prompts share) |
//! | [`covers`] | fetches and stores work cover images |
//! | [`vndb`] | vndb.org, for the cover a work's art comes from |

pub mod anki;
pub mod audio;
pub mod capture;
pub mod card;
pub mod covers;
pub mod llm;
pub mod notify;
pub mod vndb;
