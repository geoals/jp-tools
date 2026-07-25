//! A one-shot Anthropic call that writes an ultra-short "CompactDef" gloss for a
//! mined card — the sense the target word carries in its sentence, compressed to
//! something readable in under 2 seconds (~8 Japanese characters).
//!
//! Self-contained like `llm.rs`: it needs its own prompt and is called from the
//! AnkiConnect proxy after a card is added, not from the reader's explain path.
//! See `spec/anki-compactdef.md` for the reasoning behind every rule in the
//! prompt.

use std::sync::LazyLock;

use serde_json::Value;

use crate::error::AppError;
use crate::tags::{FAMILIARITY_RUBRIC, FLAVOR_RUBRIC};

/// Pinned to Opus 5. The tag-axis experiment (see `spec/anki-compactdef.md` →
/// "Why no external signals") was run on Opus 4.8 and found no thinking and no
/// external signals best; that request shape carries over to Opus 5 unchanged.
/// Opus is preferred over sonnet for the meaning/usage prose.
const MODEL: &str = "claude-opus-5";

/// Built once from the shared tag rubric (`crate::tags`) plus the CompactDef-
/// specific framing and output format. The FAMILIARITY/FLAVOR definitions live in
/// `tags.rs` so this and `llm.rs` can never drift apart again.
static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    format!(
        "\
You write a compact ENGLISH gloss (\"CompactDef\") for a Japanese vocab \
flashcard. It sits at the top of the card back as a fast recognition check; the \
full Japanese dictionary entry is shown below it. The learner is a native \
English speaker. Gloss the sense the word carries IN THE GIVEN SENTENCE.\n\n\
Output exactly two lines and nothing else — no preamble, no markdown, and never \
an XML or HTML tag of your own (do NOT write <meaning>, </meaning>, <usage>, \
<br>, or any angle-bracket tag or label):\n\
- Line 1 — the meaning, optionally followed by \". \" and one short usage note.\n\
- Line 2 — FAMILIARITY · FLAVOR[ · FLAVOR2[ · FLAVOR3]][ (structural)]\n\n\
MEANING/USAGE: nuance-carrying English. A bare one/two-word translation ONLY for \
a concrete 1-to-1 term (焼却炉 → incinerator); otherwise a short phrase that \
carries the actual nuance. Optionally one short usage note — a fixed collocation, \
a polarity restriction, or the typical speaker — where citing the Japanese word \
or its usual phrase is fine. Adult/explicit words: gloss clinically. Any Japanese \
reading you cite: hiragana, never romaji.\n\n\
{FAMILIARITY_RUBRIC}\n\n\
{FLAVOR_RUBRIC}\n\n\
STRUCTURAL (optional trailing parenthetical, orthogonal): (idiom) (mimetic) \
(fixed phrase) (proverb) (name) (four-char idiom).\n\n\
Judge from the word, the sentence, and your own knowledge ALONE — no frequency \
data, no dictionary tags. No preamble, no markdown."
    )
});

/// Generate the CompactDef gloss for `word` as used in `sentence`.
///
/// `sentence` is the plain sentence text — strip any `<b>`/furigana HTML before
/// calling so the model sees what the card shows, not markup.
pub async fn compact_def(
    http: &reqwest::Client,
    api_key: &str,
    word: &str,
    sentence: &str,
) -> Result<String, AppError> {
    let body = serde_json::json!({
        "model": MODEL,
        "max_tokens": 300,
        "thinking": { "type": "disabled" },
        "output_config": { "effort": "low" },
        "system": SYSTEM_PROMPT.as_str(),
        "messages": [{
            "role": "user",
            "content": format!("Word: {word}\nSentence: {sentence}"),
        }],
    });

    let resp = http
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("Anthropic request failed: {e}")))?;

    let status = resp.status();
    let json: Value = resp
        .json()
        .await
        .map_err(|e| AppError::Upstream(format!("Anthropic response unparseable: {e}")))?;

    if !status.is_success() {
        let msg = json["error"]["message"]
            .as_str()
            .unwrap_or("unknown API error");
        return Err(AppError::Upstream(format!(
            "Anthropic returned {status}: {msg}"
        )));
    }

    // The gloss is a short English meaning line plus a two-axis tag line;
    // clean_gloss trims stray wrapping quotes and joins the lines with <br> so
    // the tag line renders on its own line on the card back.
    extract_text(&json).map(|s| clean_gloss(&s))
}

/// Post-clean a raw gloss into the card-back HTML. The model returns a short
/// English meaning/usage block and a register keyword on a final line; this
/// trims wrapping quotes/whitespace, drops blank lines, and joins the remaining
/// lines with `<br>` (plain newlines don't render in Anki's HTML). See
/// `spec/anki-compactdef.md` for the field format.
///
/// Opus-tier models (Opus 5 especially) sometimes echo the prompt's `<meaning>`/
/// `<usage>` schema placeholders as literal tags; strip them defensively so they
/// can never reach the card even if the prompt rule is ignored.
fn clean_gloss(raw: &str) -> String {
    let raw = raw
        .replace("<meaning>", "")
        .replace("</meaning>", "")
        .replace("<usage>", "")
        .replace("</usage>", "");
    raw.split('\n')
        .map(|line| {
            line.trim()
                .trim_matches(|c: char| matches!(c, '「' | '」' | '『' | '』' | '"' | '\''))
                .trim()
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("<br>")
}

/// Pull the first text block out of an Anthropic Messages response. Finds the
/// first block of type `text` rather than blindly taking the first block: this
/// call disables thinking, but a thinking-capable model could still lead with a
/// `thinking` block if that ever changes.
fn extract_text(json: &Value) -> Result<String, AppError> {
    json["content"]
        .as_array()
        .and_then(|blocks| blocks.iter().find(|b| b["type"] == "text"))
        .and_then(|block| block["text"].as_str())
        .map(str::to_string)
        .ok_or_else(|| AppError::Upstream("no text content in Anthropic response".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_text_from_valid_response() {
        let json = serde_json::json!({
            "content": [{ "type": "text", "text": "衰える" }]
        });
        assert_eq!(extract_text(&json).unwrap(), "衰える");
    }

    #[test]
    fn extract_text_rejects_empty_content() {
        assert!(extract_text(&serde_json::json!({ "content": [] })).is_err());
        assert!(extract_text(&serde_json::json!({ "id": "msg_1" })).is_err());
    }

    #[test]
    fn clean_gloss_joins_tag_line_with_br() {
        assert_eq!(
            clean_gloss("To wane — a gradual weakening.\nCOMMON · FORMAL"),
            "To wane — a gradual weakening.<br>COMMON · FORMAL"
        );
    }

    #[test]
    fn clean_gloss_strips_wrapping_quotes_and_blank_lines() {
        assert_eq!(clean_gloss("\"incinerator\"\n\nCOMMON · PLAIN"), "incinerator<br>COMMON · PLAIN");
    }

    #[test]
    fn clean_gloss_empty_stays_empty() {
        assert_eq!(clean_gloss("   "), "");
    }

    #[test]
    fn clean_gloss_strips_literal_meaning_tags() {
        // Newline-separated (Opus 4.8 shape) with echoed placeholder tags.
        assert_eq!(
            clean_gloss("<meaning>a forgery</meaning>\nCOMMON · FORMAL"),
            "a forgery<br>COMMON · FORMAL"
        );
        // Opus 5's single-line shape: tags + its own <br> before the tag line.
        assert_eq!(
            clean_gloss("<meaning>camel</meaning><br>CORE · PLAIN"),
            "camel<br>CORE · PLAIN"
        );
    }

    #[tokio::test]
    #[ignore = "requires JP_TOOLS_ANTHROPIC_API_KEY"]
    async fn compact_def_integration() {
        let api_key = std::env::var("JP_TOOLS_ANTHROPIC_API_KEY").expect("set key");
        let http = reqwest::Client::new();
        let out = compact_def(
            &http,
            &api_key,
            "減退",
            "見た目も味も最悪な料理に食欲は減退するが、エマも口に運ぶ。",
        )
        .await
        .unwrap();
        assert!(!out.is_empty());
        assert!(out.chars().count() < 300, "should be compact, got: {out}");
        // The <meaning>/<usage> schema placeholders must never leak onto the card.
        assert!(!out.contains("<meaning>") && !out.contains("<usage>"), "tag leak: {out}");
        // Meaning line, then a two-axis tag line, joined by <br>.
        let (_, tags) = out.rsplit_once("<br>").expect("expected a <br> tag line");
        let fam = tags.split('·').next().unwrap().trim().split_whitespace().next().unwrap();
        assert!(
            ["CORE", "COMMON", "UNCOMMON", "RARE", "OBSCURE"].contains(&fam),
            "tag line should start with a familiarity token, got: {out}"
        );
        println!("CompactDef: {out}");
    }
}
