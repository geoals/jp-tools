//! A one-shot Anthropic call that writes an ultra-short "CompactDef" gloss for a
//! mined card — the sense the target word carries in its sentence, compressed to
//! something readable in under 2 seconds (~8 Japanese characters).
//!
//! Self-contained like `llm.rs`: it needs its own prompt and is called from the
//! AnkiConnect proxy after a card is added, not from the reader's explain path.
//! See `spec/anki-compactdef.md` for the reasoning behind every rule in the
//! prompt.

use serde_json::Value;

use crate::error::AppError;

const SYSTEM_PROMPT: &str = "\
You write an ultra-short gloss (\"CompactDef\") for a Japanese vocab flashcard. \
It sits at the top of the card back and must be readable in under 2 seconds — \
about 8 Japanese characters, or a few English words. It is a quick check of \
recognition, not a full definition (the full dictionary entry is shown below \
it). Gloss the sense the word carries IN THE GIVEN SENTENCE.\n\n\
Rules:\n\
- Default to Japanese. Use English ONLY when the word has a clear, direct \
English counterpart that cannot mislead — concrete nouns and established \
technical/scientific terms (e.g. 焼却炉 → incinerator). Ask yourself: could the \
English give a wrong nuance? If yes, use Japanese.\n\
- If the word is freely interchangeable with one or two close synonyms, give \
those synonyms (e.g. 弱る・衰える).\n\
- If the word carries a nuance its near-synonyms do NOT share, do NOT give a \
synonym — it would reinforce a wrong equivalence. Give a short plain-Japanese \
phrase that pins the specific sense. This applies especially to onomatopoeia, \
which usually have a specific feel rather than a synonym.\n\
- If the word is essentially the Sino-Japanese (音読み) compound form of a plain \
act, give its native 和語 counterpart (e.g. 奪取 → 奪い取る, 減退 → 衰える).\n\
- Output ONLY the gloss. No labels, no quotes, no romaji, no markdown, no \
trailing punctuation. Target under 8 Japanese characters (up to ~12 only when \
the word genuinely needs it); never a full sentence.";

/// Generate the CompactDef gloss for `word` as used in `sentence`.
///
/// `sentence` is the plain sentence text — strip any `<b>`/furigana HTML before
/// calling so the model sees what the card shows, not markup.
pub async fn compact_def(
    http: &reqwest::Client,
    api_key: &str,
    model: &str,
    word: &str,
    sentence: &str,
) -> Result<String, AppError> {
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 64,
        "system": SYSTEM_PROMPT,
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

    // Trim: the model occasionally adds a trailing newline despite the prompt.
    extract_text(&json).map(|s| s.trim().to_string())
}

/// Pull the first text block out of an Anthropic Messages response.
fn extract_text(json: &Value) -> Result<String, AppError> {
    json["content"]
        .as_array()
        .and_then(|blocks| blocks.first())
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

    #[tokio::test]
    #[ignore = "requires JP_TOOLS_ANTHROPIC_API_KEY"]
    async fn compact_def_integration() {
        let api_key = std::env::var("JP_TOOLS_ANTHROPIC_API_KEY").expect("set key");
        let http = reqwest::Client::new();
        let out = compact_def(
            &http,
            &api_key,
            "claude-haiku-4-5",
            "減退",
            "見た目も味も最悪な料理に食欲は減退するが、エマも口に運ぶ。",
        )
        .await
        .unwrap();
        assert!(!out.is_empty());
        assert!(out.chars().count() < 20, "should be compact, got: {out}");
        println!("CompactDef: {out}");
    }
}
