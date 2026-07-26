//! Which codepoints are kanji, and what is known about each one.
//!
//! The reference data lives in [`kanji_data`](super::kanji_data) and is
//! generated; this module is the interface to it. Kept beside [`super::chars`]
//! rather than in a stats crate because "is this a kanji" has to mean the same
//! thing wherever it is asked.

use super::bccwj_data::{BCCWJ, BCCWJ_TOTAL};
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

/// How common a kanji is in written Japanese at large, from the BCCWJ
/// character table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BccwjFreq {
    /// Rank among the 6937 kanji of the corpus, 1 = commonest.
    pub rank: u16,
    /// Occurrences in the corpus.
    pub count: u32,
}

impl BccwjFreq {
    /// Occurrences per million kanji — the form that compares against your own
    /// reading, where the totals are nothing alike.
    pub fn per_million(&self) -> f64 {
        self.count as f64 * 1e6 / BCCWJ_TOTAL as f64
    }
}

/// What a balanced corpus of written Japanese makes of this kanji. `None` for
/// the ones BCCWJ never saw — which, for a visual novel reader, is a real
/// answer rather than missing data: nothing in a hundred million words of
/// books, magazines, newspapers and blogs used it once.
///
/// This is a different question from [`KanjiInfo::freq`], which ranks only the
/// top 2501 of a newspaper corpus. Newspapers are one genre and stop early;
/// BCCWJ covers the whole tail, so it can tell "rare" from "unlisted".
pub fn bccwj(c: char) -> Option<BccwjFreq> {
    let i = BCCWJ.binary_search_by_key(&c, |r| r.0).ok()?;
    let (_, count, rank) = BCCWJ[i];
    Some(BccwjFreq { rank, count })
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
    fn bccwj_ranks_the_commonest_kanji_first() {
        // 人 heads the BCCWJ kanji table; 一 and 日 are the next two.
        assert_eq!(bccwj('人').unwrap().rank, 1);
        assert!(bccwj('一').unwrap().rank <= 3);
        // 人 is a bit over 1% of all kanji written in Japanese.
        assert!((10_000.0..12_000.0).contains(&bccwj('人').unwrap().per_million()));
        // Kana are not in the table, and neither are iteration marks.
        assert!(bccwj('あ').is_none());
        assert!(bccwj('々').is_none());
    }

    #[test]
    fn bccwj_reaches_past_the_newspaper_list() {
        // 邂 is far outside KANJIDIC's top-2501 newspaper ranking, but a
        // balanced corpus still saw it — that gap is the point of the table.
        assert!(info('邂').unwrap().freq.is_none());
        assert!(bccwj('邂').unwrap().rank > 2501);
    }

    #[test]
    fn joyo_set_is_2136() {
        let total: usize = JOYO_GRADES.iter().map(|&g| grade_size(g)).sum();
        assert_eq!(total, 2136);
    }
}
