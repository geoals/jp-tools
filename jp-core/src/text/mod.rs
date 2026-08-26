//! Japanese text primitives — pure functions over a string, no state and no
//! dictionary.
//!
//! These sit below [`crate::tokenize`] and [`crate::dictionary`]: they answer
//! questions about the *characters*, not about the words. Everything here is
//! shared across the tools because the answers have to agree — a character
//! counted by kotodex-server's speed figures and a character counted anywhere else
//! must be the same character.
//!
//! - [`chars`] — which codepoints count as Japanese text, matched to
//!   texthooker-ui so reading speeds are comparable with the wider community's.
//! - [`sentences`] — segmenting a block of text into sentences.
//! - [`kana`] — katakana/hiragana normalization, so a Sudachi reading and a
//!   dictionary reading are the same string.
//! - [`kanji`] — which codepoints are kanji, and the grade/frequency reference
//!   tables behind the kanji statistics: KANJIDIC's school grades and
//!   newspaper ranks, plus BCCWJ's balanced-corpus counts.

mod bccwj_data;
pub mod chars;
pub mod kana;
pub mod kanji;
mod kanji_data;
pub mod sentences;
