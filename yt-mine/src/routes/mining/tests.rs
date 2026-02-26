use super::*;
use jp_core::tokenize::Token;

#[test]
fn format_seconds_formats_correctly() {
    assert_eq!(format_seconds(0.0), "0:00");
    assert_eq!(format_seconds(5.5), "0:05");
    assert_eq!(format_seconds(65.0), "1:05");
    assert_eq!(format_seconds(3661.0), "61:01");
}

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
