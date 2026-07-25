//! Japanese text primitives — pure functions over a string, no state and no
//! dictionary.
//!
//! These sit below [`crate::tokenize`] and [`crate::dictionary`]: they answer
//! questions about the *characters*, not about the words. Everything here is
//! shared across the tools because the answers have to agree — a character
//! counted by read-stats' speed figures and a character counted anywhere else
//! must be the same character.
//!
//! - [`chars`] — which codepoints count as Japanese text, matched to
//!   texthooker-ui so reading speeds are comparable with the wider community's.
//! - [`dialogue`] — splitting a line into spoken dialogue and narration by
//!   corner-bracket depth.
//! - [`sentences`] — segmenting a block of text into sentences.
//! - [`kanji`] — which codepoints are kanji, and the grade/frequency reference
//!   table behind the kanji statistics.

pub mod chars;
pub mod dialogue;
pub mod kanji;
mod kanji_data;
pub mod sentences;
