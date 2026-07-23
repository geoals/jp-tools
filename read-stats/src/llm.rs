//! A one-shot call to Anthropic's Messages API to explain the line currently on
//! screen in the reader.
//!
//! Self-contained rather than reusing yt-mine's `LlmDefiner`: the two live in
//! different binary crates, and this needs a different prompt — explain a whole
//! line in its running context, optionally centred on a word the reader
//! selected — without dragging in the trait/mock scaffolding yt-mine wants for
//! its pipeline.

use serde_json::Value;

use crate::error::AppError;

const SYSTEM_PROMPT: &str = "\
You are a Japanese reading tutor helping an advanced learner read a visual \
novel. You are given the last few lines on screen as context; explain only the \
FINAL line.\n\n\
Open with a short, natural English rendering of the line. Then add one or two \
brief notes on nuance, grammar, or a reference a plain translation would miss; \
if a focus word is given, centre these on it (its meaning and role here).\n\n\
When a focus word is given, finish with a line starting 'Register:' that places \
the word in exactly ONE of these categories, plus a few words of \
qualification:\n\
- EVERYDAY: nearly all adults both know it and use it themselves in ordinary \
speech.\n\
- PASSIVE: nearly all adults understand it but seldom say it themselves \
(bookish-common, written register, or a fixed phrase).\n\
- FORMAL/LITERARY: educated adults know it; it belongs to writing, news, or \
formal speech rather than casual conversation.\n\
- SPECIALIZED/DATED: technical, jargon, archaic, or dialectal — many adults \
would not use it and some would not know it.\n\
- OBSCURE: many native speakers would not even recognize it.\n\n\
Whenever you give the reading of a word, write it in hiragana, never in romaji \
— and make sure it is the reading the word actually takes here (e.g. 金目のもの \
is かねめのもの, not きんめ). Be very concise: no filler, no preamble, each block \
just a line or two. You may use light Markdown — a bold label, or a short \
bullet list with one-line bullets — but nothing heavier.";

/// Ask the model to explain `context`'s last line. Earlier entries are prior
/// lines (oldest first) given only for context; `focus` is a word selected in
/// the line to centre on, or empty.
pub async fn explain(
    http: &reqwest::Client,
    api_key: &str,
    model: &str,
    context: &[String],
    focus: &str,
) -> Result<String, AppError> {
    let (earlier, target) = context.split_at(context.len() - 1);
    let target = &target[0];

    let mut user = String::new();
    if !earlier.is_empty() {
        user.push_str("Context (earlier lines, oldest first):\n");
        for line in earlier {
            user.push_str(line);
            user.push('\n');
        }
        user.push('\n');
    }
    user.push_str("Line to explain:\n");
    user.push_str(target);
    if !focus.is_empty() {
        user.push_str("\n\nFocus word: ");
        user.push_str(focus);
    }

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 512,
        "system": SYSTEM_PROMPT,
        "messages": [{ "role": "user", "content": user }],
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

    extract_text(&json)
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
            "content": [{ "type": "text", "text": "The line means X." }]
        });
        assert_eq!(extract_text(&json).unwrap(), "The line means X.");
    }

    #[test]
    fn extract_text_rejects_empty_content() {
        assert!(extract_text(&serde_json::json!({ "content": [] })).is_err());
        assert!(extract_text(&serde_json::json!({ "id": "msg_1" })).is_err());
    }
}
