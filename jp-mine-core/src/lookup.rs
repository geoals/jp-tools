use jp_core::define;
use jp_core::dictionary::wrap_definitions;
use jp_core::knowledge::Knowledge;
use jp_core::tokenize::Token;

pub struct WordLookupResult {
    pub definition_html: Option<String>,
    pub reading: String,
    pub pitch_num: Option<String>,
    /// How common the word is in fiction, by
    /// [`jp_core::knowledge::dictionaries::READER_FREQUENCY`] — 2000 is the
    /// 2000th most common word.
    pub frequency: Option<i64>,
}

/// One word, flattened into the four things a mined card needs.
///
/// [`jp_core::define`] does the asking, so a card gets the same answer the VN
/// overlay's popup shows: the dictionaries in reading order rather than install
/// order, narrowed to `reading` where one is known, and ranked by the list the
/// reader's own underline uses. Ranking on whichever dictionary answered first
/// is what this used to do, and that was BCCWJ — newspaper prose, where 船舶 is
/// the 3,843rd commonest word against 32,370th in fiction.
///
/// One sense per dictionary, which is what a card has room for. The popup is
/// where a word goes to be read in full.
pub async fn lookup_word(k: &Knowledge, word: &str, reading: Option<&str>) -> WordLookupResult {
    let found = match define::define(k.pool(), word, reading).await {
        Ok(found) => found,
        Err(e) => {
            tracing::warn!(word, error = %e, "dictionary lookup failed");
            return WordLookupResult {
                definition_html: None,
                reading: String::new(),
                pitch_num: None,
                frequency: None,
            };
        }
    };

    let def_parts: Vec<String> = found
        .sources
        .iter()
        .filter_map(|s| {
            let sense = s.senses.first()?;
            Some(wrap_definitions(
                &s.slug,
                &s.dictionary,
                &sense.definitions.join("; "),
            ))
        })
        .collect();

    WordLookupResult {
        definition_html: (!def_parts.is_empty()).then(|| def_parts.join("")),
        reading: reading.map(str::to_string).unwrap_or_else(|| {
            found
                .sources
                .iter()
                .flat_map(|s| &s.senses)
                .map(|sense| &sense.reading)
                .find(|r| !r.is_empty())
                .cloned()
                .unwrap_or_default()
        }),
        pitch_num: found.pitch.first().map(|p| {
            p.positions
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",")
        }),
        frequency: found.jiten,
    }
}

/// The target word as the line actually spelt it — conjugated, and in whatever
/// script the speaker's transcript used.
///
/// A word is picked in the UI by its base form, which is Sudachi's *normalized*
/// spelling: a transcript saying できる selects 出来る, and a kana-written ateji
/// selects its kanji. That is the right key for a dictionary lookup and the
/// wrong one to ask an LLM how common a word is — shown the kanji it rates the
/// kanji. Everything that judges the word as it was met takes this instead.
pub fn target_surface(tokens: &[Token], target_base_form: &str) -> Option<String> {
    tokens
        .iter()
        .find(|t| t.base_form == target_base_form)
        .map(|t| t.surface.clone())
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
    fn target_surface_is_what_the_line_said() {
        let tokens = vec![
            Token {
                surface: "すえた".into(),
                base_form: "饐える".into(),
                dictionary_form: "饐える".into(),
                reading: "スエ".into(),
                pos: "動詞".into(),
                proper_noun: false,
                subsidiary: false,
                counter: false,
                derived_class: None,
                inflected: true,
            },
            Token {
                surface: "臭い".into(),
                base_form: "臭い".into(),
                dictionary_form: "臭い".into(),
                reading: "ニオイ".into(),
                pos: "名詞".into(),
                proper_noun: false,
                subsidiary: false,
                counter: false,
                derived_class: None,
                inflected: false,
            },
        ];
        assert_eq!(target_surface(&tokens, "饐える").as_deref(), Some("すえた"));
        assert_eq!(target_surface(&tokens, "別の語"), None);
    }

    #[test]
    fn bold_target_wraps_matching_token() {
        let tokens = vec![
            Token {
                surface: "東京".into(),
                base_form: "東京".into(),
                dictionary_form: "東京".into(),
                reading: "トウキョウ".into(),
                pos: "名詞".into(),
                proper_noun: false,
                subsidiary: false,
                counter: false,
                derived_class: None,
                inflected: false,
            },
            Token {
                surface: "に".into(),
                base_form: "に".into(),
                dictionary_form: "に".into(),
                reading: "ニ".into(),
                pos: "助詞".into(),
                proper_noun: false,
                subsidiary: false,
                counter: false,
                derived_class: None,
                inflected: false,
            },
            Token {
                surface: "行く".into(),
                base_form: "行く".into(),
                dictionary_form: "行く".into(),
                reading: "イク".into(),
                pos: "動詞".into(),
                proper_noun: false,
                subsidiary: false,
                counter: false,
                derived_class: None,
                inflected: false,
            },
        ];
        assert_eq!(
            bold_target_in_sentence(&tokens, "行く"),
            Some("東京に<b>行く</b>".into()),
        );
    }

    #[test]
    fn bold_target_wraps_conjugated_surface() {
        let tokens = vec![
            Token {
                surface: "食べ".into(),
                base_form: "食べる".into(),
                dictionary_form: "食べる".into(),
                reading: "タベ".into(),
                pos: "動詞".into(),
                proper_noun: false,
                subsidiary: false,
                counter: false,
                derived_class: None,
                inflected: false,
            },
            Token {
                surface: "た".into(),
                base_form: "た".into(),
                dictionary_form: "た".into(),
                reading: "タ".into(),
                pos: "助動詞".into(),
                proper_noun: false,
                subsidiary: false,
                counter: false,
                derived_class: None,
                inflected: false,
            },
        ];
        assert_eq!(
            bold_target_in_sentence(&tokens, "食べる"),
            Some("<b>食べ</b>た".into()),
        );
    }

    #[test]
    fn bold_target_no_match_returns_none() {
        let tokens = vec![Token {
            surface: "テスト".into(),
            base_form: "テスト".into(),
            dictionary_form: "テスト".into(),
            reading: "テスト".into(),
            pos: "名詞".into(),
            proper_noun: false,
            subsidiary: false,
            counter: false,
            derived_class: None,
            inflected: false,
        }];
        assert_eq!(bold_target_in_sentence(&tokens, "別の語"), None);
    }

    #[test]
    fn bold_target_wraps_multiple_occurrences() {
        let tokens = vec![
            Token {
                surface: "食べ".into(),
                base_form: "食べる".into(),
                dictionary_form: "食べる".into(),
                reading: "タベ".into(),
                pos: "動詞".into(),
                proper_noun: false,
                subsidiary: false,
                counter: false,
                derived_class: None,
                inflected: false,
            },
            Token {
                surface: "て".into(),
                base_form: "て".into(),
                dictionary_form: "て".into(),
                reading: "テ".into(),
                pos: "助詞".into(),
                proper_noun: false,
                subsidiary: false,
                counter: false,
                derived_class: None,
                inflected: false,
            },
            Token {
                surface: "食べ".into(),
                base_form: "食べる".into(),
                dictionary_form: "食べる".into(),
                reading: "タベ".into(),
                pos: "動詞".into(),
                proper_noun: false,
                subsidiary: false,
                counter: false,
                derived_class: None,
                inflected: false,
            },
            Token {
                surface: "た".into(),
                base_form: "た".into(),
                dictionary_form: "た".into(),
                reading: "タ".into(),
                pos: "助動詞".into(),
                proper_noun: false,
                subsidiary: false,
                counter: false,
                derived_class: None,
                inflected: false,
            },
        ];
        assert_eq!(
            bold_target_in_sentence(&tokens, "食べる"),
            Some("<b>食べ</b>て<b>食べ</b>た".into()),
        );
    }
}
