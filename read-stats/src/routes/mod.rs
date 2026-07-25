//! HTTP handlers, one module per resource.
//!
//! Handlers are deliberately thin: parse the query, ask [`crate::history`] or
//! [`crate::db`] for rows, shape the JSON. Anything that decides *what a number
//! means* belongs in [`crate::stats`], where it can be tested without a server.
//!
//! | module | endpoints |
//! |---|---|
//! | [`summary`] | `/api/summary` — today, the goal meter, the streak |
//! | [`days`] | `/api/days` — the daily bar/trend series |
//! | [`timeline`] | `/api/day/timeline` — one day's intra-day curve |
//! | [`sessions`] | `/api/sessions` — derived sittings + manual entries |
//! | [`works`] | `/api/works` — per-VN totals and metadata |
//! | [`dialogue`] | `/api/dialogue/summary` — speech vs prose |
//! | [`lookups`] | `/api/lookups/summary` — the mining funnel |
//! | [`anki`] | `/api/anki/*` — deck snapshot + re-encounter stats |
//! | [`settings`] | `/api/settings`, `/api/pause` |
//! | [`reader`] | the phone-side `#read` view: line feed, mine, explain |

pub mod anki;
pub mod ankiproxy;
pub mod days;
pub mod dialogue;
pub mod json;
pub mod lookups;
pub mod reader;
pub mod sessions;
pub mod settings;
pub mod summary;
pub mod timeline;
pub mod works;
