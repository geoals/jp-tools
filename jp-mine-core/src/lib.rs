//! Shared back half of the sentence-mining pipeline: dictionary lookup,
//! card formatting, and Anki export. Used by both `yt-mine` and `manga-mine`.

pub mod card;
pub mod compactdef;
pub mod config;
pub mod export;
pub mod llm;
pub mod localaudio;
pub mod lookup;
pub mod tags;
