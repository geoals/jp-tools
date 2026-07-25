//! Which codepoints are kanji, and what is known about each one.
//!
//! The reference data lives in [`kanji_data`](super::kanji_data) and is
//! generated; this module is the interface to it. Kept beside [`super::chars`]
//! rather than in a stats crate because "is this a kanji" has to mean the same
//! thing wherever it is asked.

use super::kanji_data::KANJI;

/// True for CJK unified ideographs — the ranges [`super::chars::is_counted`]
/// keeps, minus kana, radicals and latin. Iteration marks (々〆) are *not*
/// kanji here: they stand in for a kanji rather than being one, and counting
/// them would put a meaningless glyph in the grid.
pub fn is_kanji(c: char) -> bool {
    matches!(c,
        '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{20000}'..='\u{2A6DF}' | '\u{2A700}'..='\u{2EE5D}'
        | '\u{30000}'..='\u{33479}'
    )
}

/// What the reference table knows about one kanji.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KanjiInfo {
    /// 1-6 primary school, 8 the rest of the jōyō set, 9-10 jinmeiyō, `None`
    /// unlisted.
    pub grade: Option<u8>,
    pub strokes: Option<u8>,
    /// Rank in a newspaper corpus, 1 = most common. `None` outside the top 2501.
    pub freq: Option<u16>,
    /// One English gloss, enough to recognise the kanji in a tooltip.
    pub gloss: &'static str,
}

/// The reference row for a kanji, if it has one. Plenty of kanji that turn up
/// in visual novels do not.
pub fn info(c: char) -> Option<KanjiInfo> {
    let i = KANJI.binary_search_by_key(&c, |r| r.0).ok()?;
    let (_, grade, strokes, freq, gloss) = KANJI[i];
    Some(KanjiInfo {
        grade: (grade != 0).then_some(grade),
        strokes: (strokes != 0).then_some(strokes),
        freq: (freq != 0).then_some(freq),
        gloss,
    })
}

/// The jōyō grades, in teaching order. 8 is the secondary-school remainder —
/// over half the set, which is why it is worth showing as its own band rather
/// than folded into a single jōyō percentage.
pub const JOYO_GRADES: [u8; 7] = [1, 2, 3, 4, 5, 6, 8];

/// How many kanji the reference table lists for a grade — the denominator for
/// coverage.
pub fn grade_size(grade: u8) -> usize {
    KANJI.iter().filter(|r| r.1 == grade).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_kanji_against_kana_and_marks() {
        assert!(is_kanji('漢'));
        assert!(!is_kanji('か'));
        assert!(!is_kanji('ー'));
        assert!(!is_kanji('々'));
    }

    #[test]
    fn looks_up_reference_rows() {
        let one = info('一').unwrap();
        assert_eq!(one.grade, Some(1));
        assert_eq!(one.strokes, Some(1));
        assert!(one.freq.unwrap() < 10);
        assert!(info('あ').is_none());
    }

    #[test]
    fn joyo_set_is_2136() {
        let total: usize = JOYO_GRADES.iter().map(|&g| grade_size(g)).sum();
        assert_eq!(total, 2136);
    }
}
