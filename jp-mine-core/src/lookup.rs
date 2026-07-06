use std::sync::Arc;

use jp_core::dictionary::Dictionary;
use jp_core::tokenize::Token;

pub struct WordLookupResult {
    pub definition_html: Option<String>,
    pub reading: String,
    pub pitch_num: Option<String>,
    /// Best (lowest) frequency rank across dictionaries, e.g. 2000 = the
    /// 2000th most common word.
    pub frequency: Option<i64>,
}

/// Look a word up across all configured dictionaries. Definitions from every
/// dictionary that has the word are concatenated (each wrapped in its
/// dictionary's styling); reading, pitch and frequency come from the first hit.
pub async fn lookup_word(dictionaries: &[Arc<Dictionary>], word: &str) -> WordLookupResult {
    let mut def_parts = Vec::new();
    let mut reading = String::new();
    let mut pitch_num = None;
    let mut frequency = None;

    for dict in dictionaries {
        let entries = dict.lookup(word).await;
        if let Some(entry) = entries.first() {
            let joined = entry.definitions.join("; ");
            def_parts.push(dict.wrap_definitions(&joined));
            if reading.is_empty() && !entry.reading.is_empty() {
                reading = entry.reading.clone();
            }
        }
        if pitch_num.is_none() {
            let pitch = dict.lookup_pitch(word).await;
            if !pitch.is_empty() {
                let nums: Vec<String> = pitch[0]
                    .positions
                    .iter()
                    .map(|p| p.to_string())
                    .collect();
                pitch_num = Some(nums.join(","));
            }
        }
        if frequency.is_none() {
            frequency = dict.lookup_frequency(word).await;
        }
    }

    WordLookupResult {
        definition_html: if def_parts.is_empty() {
            None
        } else {
            Some(def_parts.join(""))
        },
        reading,
        pitch_num,
        frequency,
    }
}

/// Build sentence HTML with the target word's surface form(s) wrapped in `<b></b>`.
pub fn bold_target_in_sentence(tokens: &[Token], target_base_form: &str) -> Option<String> {
    if !tokens.iter().any(|t| t.base_form == target_base_form) {
        return None;
    }
    let mut result = String::new();
    for token in tokens {
        if token.base_form == target_base_form {
            result.push_str("<b>");
            result.push_str(&token.surface);
            result.push_str("</b>");
        } else {
            result.push_str(&token.surface);
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold_target_wraps_matching_token() {
        let tokens = vec![
            Token { surface: "東京".into(), base_form: "東京".into(), reading: "トウキョウ".into(), pos: "名詞".into() },
            Token { surface: "に".into(), base_form: "に".into(), reading: "ニ".into(), pos: "助詞".into() },
            Token { surface: "行く".into(), base_form: "行く".into(), reading: "イク".into(), pos: "動詞".into() },
        ];
        assert_eq!(
            bold_target_in_sentence(&tokens, "行く"),
            Some("東京に<b>行く</b>".into()),
        );
    }

    #[test]
    fn bold_target_wraps_conjugated_surface() {
        let tokens = vec![
            Token { surface: "食べ".into(), base_form: "食べる".into(), reading: "タベ".into(), pos: "動詞".into() },
            Token { surface: "た".into(), base_form: "た".into(), reading: "タ".into(), pos: "助動詞".into() },
        ];
        assert_eq!(
            bold_target_in_sentence(&tokens, "食べる"),
            Some("<b>食べ</b>た".into()),
        );
    }

    #[test]
    fn bold_target_no_match_returns_none() {
        let tokens = vec![
            Token { surface: "テスト".into(), base_form: "テスト".into(), reading: "テスト".into(), pos: "名詞".into() },
        ];
        assert_eq!(bold_target_in_sentence(&tokens, "別の語"), None);
    }

    #[test]
    fn bold_target_wraps_multiple_occurrences() {
        let tokens = vec![
            Token { surface: "食べ".into(), base_form: "食べる".into(), reading: "タベ".into(), pos: "動詞".into() },
            Token { surface: "て".into(), base_form: "て".into(), reading: "テ".into(), pos: "助詞".into() },
            Token { surface: "食べ".into(), base_form: "食べる".into(), reading: "タベ".into(), pos: "動詞".into() },
            Token { surface: "た".into(), base_form: "た".into(), reading: "タ".into(), pos: "助動詞".into() },
        ];
        assert_eq!(
            bold_target_in_sentence(&tokens, "食べる"),
            Some("<b>食べ</b>て<b>食べ</b>た".into()),
        );
    }
}
