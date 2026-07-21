//! Character counting, matched to texthooker-ui so speeds are comparable.
//!
//! texthooker-ui counts a line as `text.replace(isNotJapaneseRegex, '')`
//! codepoints, where
//!
//! ```text
//! /[^0-9A-Z○◯々-〇〻ぁ-ゖゝ-ゞァ-ヺー０-９Ａ-Ｚｦ-ﾝ\p{Radical}\p{Unified_Ideograph}]+/gimu
//! ```
//!
//! It is an allowlist, so punctuation, brackets, whitespace and the like all
//! drop out — counting raw codepoints instead inflates chars/h by roughly the
//! punctuation share of the text (~10-20% for VN prose).

/// True for codepoints texthooker-ui keeps.
pub fn is_counted(c: char) -> bool {
    matches!(c,
        '0'..='9' | 'A'..='Z' | 'a'..='z'    // the /i flag makes A-Z cover a-z
        | '\u{25CB}' | '\u{25EF}'            // ○ ◯
        | '\u{3005}'..='\u{3007}'            // 々 〆 〇
        | '\u{303B}'                         // 〻
        | '\u{3041}'..='\u{3096}'            // ぁ-ゖ
        | '\u{309D}'..='\u{309E}'            // ゝ ゞ
        | '\u{30A1}'..='\u{30FA}'            // ァ-ヺ
        | '\u{30FC}'                         // ー
        | '\u{FF10}'..='\u{FF19}'            // ０-９
        | '\u{FF21}'..='\u{FF3A}' | '\u{FF41}'..='\u{FF5A}'  // Ａ-Ｚ ａ-ｚ
        | '\u{FF66}'..='\u{FF9D}'            // ｦ-ﾝ (halfwidth katakana)
        // \p{Radical}
        | '\u{2E80}'..='\u{2E99}' | '\u{2E9B}'..='\u{2EF3}' | '\u{2F00}'..='\u{2FD5}'
        // \p{Unified_Ideograph}
        | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}'
        | '\u{FA0E}'..='\u{FA0F}' | '\u{FA11}' | '\u{FA13}'..='\u{FA14}' | '\u{FA1F}'
        | '\u{FA21}' | '\u{FA23}'..='\u{FA24}' | '\u{FA27}'..='\u{FA29}'
        | '\u{20000}'..='\u{2A6DF}' | '\u{2A700}'..='\u{2B81D}' | '\u{2B820}'..='\u{2CEAD}'
        | '\u{2CEB0}'..='\u{2EBE0}' | '\u{2EBF0}'..='\u{2EE5D}'
        | '\u{30000}'..='\u{3134A}' | '\u{31350}'..='\u{33479}'
    )
}

pub fn count_chars(text: &str) -> i64 {
    text.chars().filter(|&c| is_counted(c)).count() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_punctuation_and_whitespace() {
        assert_eq!(count_chars("「ねえ、聞いてる？」"), 6);
        assert_eq!(count_chars("……そうか。"), 3);
        assert_eq!(count_chars("　 \n"), 0);
    }

    #[test]
    fn keeps_kana_kanji_and_alphanumerics() {
        assert_eq!(count_chars("漢字かなカナ"), 6);
        assert_eq!(count_chars("ＡＢ12ab"), 6);
        assert_eq!(count_chars("々〆〇ゝヽー"), 5); // ヽ (U+30FD) is outside ァ-ヺ
        assert_eq!(count_chars("ﾊﾛｰ"), 3);
    }

    #[test]
    fn counts_by_codepoint_not_byte() {
        assert_eq!(count_chars("𠮟る"), 2); // surrogate-pair kanji from Ext B
    }
}
