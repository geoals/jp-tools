//! Kana normalization — one spelling of a reading, so two sources agree.
//!
//! Readings arrive in two alphabets. Sudachi's `reading_form()` is katakana
//! (ヨム), every Yomitan dictionary's reading field is hiragana (よむ). The
//! vocabulary ledger is keyed on `(headword, reading)` and joined against the
//! dictionaries by that key, so one of the two has to give. Hiragana wins: it
//! is what the dictionaries hold, what the ledger is displayed in, and the
//! only one of the pair a katakana loanword's reading can't be confused with.
//!
//! This is a codepoint shift and nothing more — no long-vowel expansion, no
//! romaji. ー stays ー, because in a katakana reading it *is* the reading.

/// Katakana → hiragana, leaving everything else (kanji, ー, ヶ, punctuation,
/// latin) exactly as it was.
///
/// Only the main block ァ..ヶ is shifted. ヷヸヹヺ and the halfwidth forms have
/// no hiragana counterpart to shift to and are left alone rather than mangled.
pub fn to_hiragana(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'ァ'..='ヶ' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// Whether a character is hiragana.
///
/// Used to decide whether a one-character piece of a compound may stand as a
/// word: hiragana may (a lone と is a particle, which ingest drops on part of
/// speech), katakana may not (it shreds names — see
/// [`crate::tokenize::SudachiTokenizer::recompose`]).
pub fn is_hiragana(c: char) -> bool {
    matches!(c, 'ぁ'..='ゖ')
}

/// Whether every character is kana (either alphabet), ー and ・ included.
///
/// The ledger uses this to decide whether a headword needs a reading beside
/// it at all: for a kana-only word the two are the same string, so the reading
/// is stored empty rather than duplicated — see
/// [`crate::knowledge::vocabulary::Term::new`].
pub fn is_all_kana(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| matches!(c, 'ぁ'..='ゖ' | 'ァ'..='ヶ' | 'ー' | '・' | 'ﾞ' | 'ﾟ' | '゛' | '゜'))
}

/// Whether every character is katakana, ー and ・ included.
pub fn is_all_katakana(s: &str) -> bool {
    is_all_kana(s) && !s.chars().any(is_hiragana)
}

/// Whether every character is hiragana, ー and ・ included.
pub fn is_all_hiragana(s: &str) -> bool {
    is_all_kana(s) && !s.chars().any(|c| matches!(c, 'ァ'..='ヶ'))
}

/// Beats of speech. A small ゃゅょ rides on the kana before it; everything else,
/// っ and ん included, is its own beat.
///
/// The unit several rules are stated in — how much of a word a kana surface
/// actually spells — so it lives here rather than beside any one of them.
pub fn morae(reading: &str) -> usize {
    reading
        .chars()
        .filter(|c| !matches!(c, 'ゃ' | 'ゅ' | 'ょ' | 'ャ' | 'ュ' | 'ョ' | 'ゎ' | 'ヮ'))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beats_of_speech() {
        assert_eq!(morae("きょう"), 2);
        assert_eq!(morae("がっこう"), 4);
        assert_eq!(morae("ん"), 1);
    }

    #[test]
    fn katakana_becomes_hiragana() {
        assert_eq!(to_hiragana("ヨム"), "よむ");
        assert_eq!(to_hiragana("シンパイ"), "しんぱい");
        // Small kana and ヶ are inside the shifted range.
        assert_eq!(to_hiragana("ジッカ"), "じっか");
    }

    #[test]
    fn everything_else_survives() {
        assert_eq!(to_hiragana("読む"), "読む");
        assert_eq!(to_hiragana("よむ"), "よむ");
        // A long-vowel mark in a katakana reading is part of the reading.
        assert_eq!(to_hiragana("コーヒー"), "こーひー");
        assert_eq!(to_hiragana("ABC 123"), "ABC 123");
    }

    #[test]
    fn kana_only_words_are_recognized() {
        assert!(is_all_kana("ください"));
        assert!(is_all_kana("コーヒー"));
        assert!(!is_all_kana("読む"));
        assert!(!is_all_kana(""));
    }

    #[test]
    fn one_alphabet_or_the_other() {
        assert!(is_all_katakana("コーヒー"));
        assert!(!is_all_katakana("ざる"));
        assert!(!is_all_katakana("ハイ会社"));
        assert!(is_all_hiragana("ざる"));
        assert!(!is_all_hiragana("ザル"));
    }
}
