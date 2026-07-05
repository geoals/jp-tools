//! Shared back half of the sentence-mining pipeline: dictionary lookup,
//! card formatting, and Anki export. Used by both `yt-mine` and `manga-mine`.

pub mod config;
pub mod export;
pub mod lookup;
