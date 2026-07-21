//! Sentence segmentation for OCR'd text.

/// Split Japanese text into sentences on 。！？…‥ (and ASCII !?), keeping the
/// delimiter. Closing quotes/brackets directly after a delimiter stay attached
/// to the preceding sentence. Newlines also act as boundaries (manga bubbles
/// often lack final punctuation).
pub fn split_sentences(text: &str) -> Vec<String> {
    const DELIMITERS: &[char] = &['。', '！', '？', '!', '?', '…', '‥'];
    const TRAILERS: &[char] = &['」', '』', '）', ')', '"', '\u{201D}'];

    let mut sentences = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\n' || c == '\r' {
            push_sentence(&mut sentences, &mut current);
            continue;
        }
        current.push(c);
        if DELIMITERS.contains(&c) {
            // Consume runs of delimiters (？？, ！？, ……) and trailing quotes
            while let Some(&next) = chars.peek() {
                if DELIMITERS.contains(&next) || TRAILERS.contains(&next) {
                    current.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            push_sentence(&mut sentences, &mut current);
        }
    }
    push_sentence(&mut sentences, &mut current);

    sentences
}

fn push_sentence(sentences: &mut Vec<String>, current: &mut String) {
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        sentences.push(trimmed.to_string());
    }
    current.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_periods() {
        assert_eq!(
            split_sentences("今日は暑い。明日は寒い。"),
            vec!["今日は暑い。", "明日は寒い。"],
        );
    }

    #[test]
    fn keeps_text_without_delimiter_as_one_sentence() {
        assert_eq!(
            split_sentences("よろしくお願いします"),
            vec!["よろしくお願いします"]
        );
    }

    #[test]
    fn splits_on_question_and_exclamation() {
        assert_eq!(
            split_sentences("本当？すごい！やった"),
            vec!["本当？", "すごい！", "やった"],
        );
    }

    #[test]
    fn groups_delimiter_runs() {
        assert_eq!(
            split_sentences("なに！？そんな……まさか"),
            vec!["なに！？", "そんな……", "まさか"],
        );
    }

    #[test]
    fn keeps_closing_quote_with_sentence() {
        assert_eq!(
            split_sentences("「行くぞ！」と言った。"),
            vec!["「行くぞ！」", "と言った。"],
        );
    }

    #[test]
    fn splits_on_newlines() {
        assert_eq!(split_sentences("一行目\n二行目"), vec!["一行目", "二行目"],);
    }

    #[test]
    fn empty_input_yields_no_sentences() {
        assert_eq!(split_sentences(""), Vec::<String>::new());
        assert_eq!(split_sentences("  \n "), Vec::<String>::new());
    }
}
