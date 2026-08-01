//! A one-shot call to Anthropic's Messages API to explain the line currently on
//! screen in the reader.
//!
//! Self-contained rather than reusing yt-mine's `LlmDefiner`: the two live in
//! different binary crates, and this needs a different prompt — explain a whole
//! line in its running context, optionally centred on a word the reader
//! selected — without dragging in the trait/mock scaffolding yt-mine wants for
//! its pipeline.

use std::sync::LazyLock;

use serde_json::Value;

use crate::error::AppError;
use crate::services::tags::{FAMILIARITY_RUBRIC, FLAVOR_RUBRIC};

/// Built once from the shared tag rubric (`crate::tags`) plus the explain-path
/// framing. The FAMILIARITY/FLAVOR definitions are the same source of truth
/// `compactdef.rs` uses, so the two paths tag identically.
static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    format!(
        "\
You are a Japanese reading tutor helping an advanced learner read a visual \
novel. You are given the last few lines on screen as context; explain only the \
FINAL line.\n\n\
Open with a short, natural English rendering of the line. Then add one or two \
brief notes on nuance, grammar, or a reference a plain translation would miss; \
if a focus word is given, centre these on it (its meaning and role here).\n\n\
When a focus word is given, finish with a line starting 'Tags:' that rates the \
focus word on the two axes below, then a few words of qualification. Write it as \
'Tags: FAMILIARITY · FLAVOR[ · FLAVOR2]' followed by the qualification.\n\n\
{FAMILIARITY_RUBRIC}\n\n\
{FLAVOR_RUBRIC}\n\n\
Whenever you give the reading of a word, write it in hiragana, never in romaji \
— and make sure it is the reading the word actually takes here (e.g. 金目のもの \
is かねめのもの, not きんめ). Be very concise: no filler, no preamble, each block \
just a line or two. You may use light Markdown — a bold label, or a short \
bullet list with one-line bullets — but nothing heavier."
    )
});

/// Pinned to Opus 5 — the explain button is a short interactive lookup, and its
/// two-axis tags should match the cards' (both on Opus, thinking off). Not driven
/// by `JP_TOOLS_LLM_MODEL` (which yt-mine still uses for its own definer).
const MODEL: &str = "claude-opus-5";

/// Ask the model to explain `context`'s last line. Earlier entries are prior
/// lines (oldest first) given only for context; `focus` is a word selected in
/// the line to centre on, or empty.
pub async fn explain(
    http: &reqwest::Client,
    api_key: &str,
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

    // Thinking off: keeps this interactive helper snappy, and avoids the upward
    // familiarity bias thinking introduces into the two-axis tags. Sonnet 5 and
    // Opus 5 otherwise default to adaptive thinking when `thinking` is omitted.
    let body = serde_json::json!({
        "model": MODEL,
        "max_tokens": 512,
        "thinking": { "type": "disabled" },
        "system": SYSTEM_PROMPT.as_str(),
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

/// Pull the first text block out of an Anthropic Messages response. Finds the
/// first block of type `text` rather than blindly taking the first block, since
/// thinking-capable models (Sonnet 5, Opus 5) can emit a leading `thinking`
/// block ahead of the text.
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
            "content": [{ "type": "text", "text": "The line means X." }]
        });
        assert_eq!(extract_text(&json).unwrap(), "The line means X.");
    }

    #[test]
    fn extract_text_rejects_empty_content() {
        assert!(extract_text(&serde_json::json!({ "content": [] })).is_err());
        assert!(extract_text(&serde_json::json!({ "id": "msg_1" })).is_err());
    }

    #[test]
    fn extract_text_skips_leading_thinking_block() {
        let json = serde_json::json!({
            "content": [
                { "type": "thinking", "thinking": "" },
                { "type": "text", "text": "The line means X." }
            ]
        });
        assert_eq!(extract_text(&json).unwrap(), "The line means X.");
    }

    #[tokio::test]
    #[ignore = "requires JP_TOOLS_ANTHROPIC_API_KEY"]
    async fn explain_uses_two_axis_tags() {
        let api_key = std::env::var("JP_TOOLS_ANTHROPIC_API_KEY").expect("set key");
        let http = reqwest::Client::new();
        let context = ["「承知いたしました」と彼は恭しく頭を下げた。".to_string()];
        let out = explain(&http, &api_key, &context, "承知").await.unwrap();
        println!("Explain:\n{out}");
        // A focus word was given, so it must end with the two-axis Tags line.
        let tags = out
            .lines()
            .rev()
            .find(|l| l.contains("Tags:"))
            .expect("no Tags: line");
        assert!(
            ["CORE", "COMMON", "UNCOMMON", "RARE", "OBSCURE"]
                .iter()
                .any(|f| tags.contains(f)),
            "Tags line should carry a familiarity token, got: {tags}"
        );
        // Old single-register scheme must be gone.
        assert!(
            !out.contains("Register:"),
            "still emitting old Register line: {out}"
        );
    }
}
