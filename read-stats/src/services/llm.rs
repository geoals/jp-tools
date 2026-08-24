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
use jp_mine_core::tags::{FAMILIARITY_RUBRIC, FLAVOR_RUBRIC};

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
Every word you name — the focus word, and any kanji word the notes single out \
— gets its reading in parentheses after it.\n\n\
When a focus word is given, finish with a line starting 'Tags:' that rates the \
focus word on the two axes below, then a few words of qualification. Write it as \
'Tags: FAMILIARITY · FLAVOR[ · FLAVOR2]' followed by the qualification.\n\n\
{FAMILIARITY_RUBRIC}\n\n\
{FLAVOR_RUBRIC}\n\n\
A reading is always hiragana, never romaji, and is the reading the word takes \
here (金目のもの is かねめのもの, not きんめ). Be very concise: no filler, no preamble, each block \
just a line or two. You may use light Markdown — a bold label, or a short \
bullet list with one-line bullets — but nothing heavier."
    )
});

/// Pinned to Sonnet 5 — the explain button is a short interactive lookup read
/// once and thrown away, so it does not need the model the cards are written
/// with. Not driven by `KOTODEX_LLM_MODEL`.
///
/// The tags it prints therefore come off a different model than
/// `compactdef`'s, which is why the rubric is shared source and not a
/// paraphrase: the wording is the only thing holding the two axes together.
const MODEL: &str = "claude-sonnet-5";

/// The prompt for one explain call: earlier lines (oldest first) as context,
/// the last one as the line to explain, and `focus` — a word to centre on, or
/// empty.
fn user_message(context: &[String], focus: &str) -> String {
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
    user
}

fn request_body(user: String, stream: bool) -> Value {
    // Thinking off and effort medium: keeps this interactive helper snappy, and
    // avoids the upward familiarity bias thinking introduces into the two-axis
    // tags. Sonnet 5 defaults to adaptive thinking when `thinking` is omitted,
    // and to `high` effort — which is most of what an explain call costs, since
    // output bills at five times input.
    //
    // The cap is a cost bound, not a target: an answer this prompt asks to be
    // "very concise" runs well under it, and hitting it truncates mid-sentence.
    serde_json::json!({
        "model": MODEL,
        "max_tokens": 400,
        "thinking": { "type": "disabled" },
        "output_config": { "effort": "medium" },
        "stream": stream,
        "system": SYSTEM_PROMPT.as_str(),
        "messages": [{ "role": "user", "content": user }],
    })
}

fn send(
    http: &reqwest::Client,
    api_key: &str,
    body: Value,
    // `use<>`: the builder owns everything it needs by the time `send` is called,
    // so the future borrows nothing — without this it inherits the arguments'
    // lifetimes and cannot outlive the handler that made it.
) -> impl std::future::Future<Output = reqwest::Result<reqwest::Response>> + Send + 'static + use<>
{
    http.post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
}

/// Ask the model to explain `context`'s last line, a piece at a time.
///
/// Streamed rather than awaited whole: the answer is a few hundred tokens off
/// Opus, and unstreamed that is several seconds of nothing on screen while the
/// reader waits mid-line. The model is pinned, so the only lever on how long
/// this *feels* is when the first words arrive.
pub fn explain_stream(
    http: &reqwest::Client,
    api_key: &str,
    context: &[String],
    focus: &str,
) -> impl futures_util::Stream<Item = Result<String, AppError>> + Send + 'static + use<> {
    deltas(send(
        http,
        api_key,
        request_body(user_message(context, focus), true),
    ))
}

/// The text deltas of an in-flight Messages request.
///
/// Split from `explain_stream` so the request — the only thing here that
/// borrows — is built and owned before the stream body exists. Written inline,
/// `stream!` captures the caller's lifetimes whether it uses them or not, and
/// the result cannot then be handed to a response.
fn deltas(
    request: impl std::future::Future<Output = reqwest::Result<reqwest::Response>> + Send + 'static,
) -> impl futures_util::Stream<Item = Result<String, AppError>> + Send + 'static {
    async_stream::stream! {
        let resp = match request.await {
            Ok(r) => r,
            Err(e) => {
                yield Err(AppError::Upstream(format!("Anthropic request failed: {e}")));
                return;
            }
        };

        let status = resp.status();
        if !status.is_success() {
            // An error answers as one JSON body, not as an event stream.
            let json: Value = resp.json().await.unwrap_or_default();
            let msg = json["error"]["message"].as_str().unwrap_or("unknown API error");
            yield Err(AppError::Upstream(format!("Anthropic returned {status}: {msg}")));
            return;
        }

        let mut body = resp.bytes_stream();
        // Chunks split anywhere, including mid-line and mid-UTF-8, so frames are
        // reassembled here rather than decoded per chunk.
        let mut buf = Vec::new();
        while let Some(chunk) = futures_util::StreamExt::next(&mut body).await {
            match chunk {
                Ok(bytes) => buf.extend_from_slice(&bytes),
                Err(e) => {
                    yield Err(AppError::Upstream(format!("Anthropic stream broke: {e}")));
                    return;
                }
            }
            while let Some(nl) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=nl).collect();
                if let Some(text) = text_delta(&line) {
                    yield Ok(text);
                }
            }
        }
    }
}

/// The text a `content_block_delta` frame carries, or nothing — every other
/// line of the stream is an event name, a blank separator, or a frame about
/// something else (message start, usage, block boundaries).
fn text_delta(line: &[u8]) -> Option<String> {
    let line = std::str::from_utf8(line).ok()?.trim();
    let payload = line.strip_prefix("data:")?.trim();
    let json: Value = serde_json::from_str(payload).ok()?;
    if json["type"] != "content_block_delta" || json["delta"]["type"] != "text_delta" {
        return None;
    }
    Some(json["delta"]["text"].as_str()?.to_string())
}

/// The whole explanation, awaited. Only the tests use this — the reader and the
/// overlay both stream — but it is the same request and prompt, so what it
/// asserts holds for what they get.
#[cfg(test)]
pub async fn explain(
    http: &reqwest::Client,
    api_key: &str,
    context: &[String],
    focus: &str,
) -> Result<String, AppError> {
    let resp = send(
        http,
        api_key,
        request_body(user_message(context, focus), false),
    )
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
#[cfg(test)]
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
    #[ignore = "requires KOTODEX_ANTHROPIC_API_KEY"]
    async fn explain_uses_two_axis_tags() {
        let api_key = std::env::var("KOTODEX_ANTHROPIC_API_KEY").expect("set key");
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
