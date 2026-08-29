//! Which model answers, and how to ask it.
//!
//! Two request shapes reach every service worth pointing this at: Anthropic's
//! Messages API, and the OpenAI chat-completions shape that OpenAI, OpenRouter,
//! DeepSeek, Gemini's compatibility endpoint and a local llama.cpp or Ollama all
//! speak. **Two adapters and no more** — a third would be a provider that
//! answers neither shape, and there are few enough of those to wait for one.
//!
//! One implementation, because the two prompts that use it — the card gloss in
//! [`crate::compactdef`] and the reader's line explanation in kotodex-server —
//! are the same request with different words in it. Which model each asks for is
//! still their own: the gloss wants the best model available and the explanation
//! is read once and thrown away.

use futures_util::{Stream, StreamExt};
use serde_json::{Value, json};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Failed(String),
    /// Nothing is configured, or nothing is listening. Separate from [`Error::Failed`]
    /// because it is not about this request: a caller looping over thousands of
    /// cards must stop rather than fail each one in turn.
    #[error("{0}")]
    Unavailable(String),
}

/// Which request shape the endpoint speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Anthropic,
    OpenAi,
}

impl Kind {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "anthropic" => Some(Kind::Anthropic),
            "openai" => Some(Kind::OpenAi),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Anthropic => "anthropic",
            Kind::OpenAi => "openai",
        }
    }

    fn base_url(self) -> &'static str {
        match self {
            Kind::Anthropic => "https://api.anthropic.com",
            Kind::OpenAi => "https://api.openai.com/v1",
        }
    }

    fn path(self) -> &'static str {
        match self {
            Kind::Anthropic => "/v1/messages",
            Kind::OpenAi => "/chat/completions",
        }
    }
}

/// Where the model is, and which key opens it.
#[derive(Debug, Clone)]
pub struct Provider {
    pub kind: Kind,
    /// Empty for [`Kind`]'s own. An OpenAI-shaped base URL includes the version
    /// segment, because that is where every such service puts it and they do not
    /// agree on what it is called.
    pub base_url: String,
    /// Empty to leave each prompt on the model it was tuned against. One name
    /// here would price the gloss and the line explanation the same, and they
    /// are not worth the same.
    pub model: String,
    pub api_key: String,
}

/// One request, whichever shape it goes out in.
pub struct Ask<'a> {
    pub system: &'a str,
    pub messages: &'a [Value],
    pub max_tokens: u32,
    /// Used when the provider names no model of its own.
    pub default_model: &'a str,
    /// Ask for the system block to be cached. Only Anthropic prices it
    /// separately; the OpenAI shape has no field for it and caches or does not
    /// on its own.
    pub cache_system: bool,
}

impl Provider {
    /// Anthropic behind `KOTODEX_ANTHROPIC_API_KEY`.
    ///
    /// For the callers with no settings database to read a provider out of: yt-mine,
    /// and the examples that tune the tag rubric. kotodex-server resolves its own
    /// from `settings`, which is where a reader sets one.
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("KOTODEX_ANTHROPIC_API_KEY").ok()?;
        (!api_key.trim().is_empty()).then(|| Provider {
            kind: Kind::Anthropic,
            base_url: String::new(),
            model: String::new(),
            api_key,
        })
    }

    /// The provider's own model, or the prompt's default.
    fn model_for(&self, ask: &Ask<'_>) -> String {
        if self.model.trim().is_empty() {
            ask.default_model.to_string()
        } else {
            self.model.trim().to_string()
        }
    }

    fn endpoint(&self) -> String {
        let base = self.base_url.trim().trim_end_matches('/');
        let base = if base.is_empty() {
            self.kind.base_url()
        } else {
            base
        };
        format!("{base}{}", self.kind.path())
    }

    fn body(&self, ask: &Ask<'_>, stream: bool) -> Value {
        let model = self.model_for(ask);
        match self.kind {
            Kind::Anthropic => {
                let system = if ask.cache_system {
                    json!([{
                        "type": "text",
                        "text": ask.system,
                        "cache_control": { "type": "ephemeral" },
                    }])
                } else {
                    json!(ask.system)
                };
                json!({
                    "model": model,
                    "max_tokens": ask.max_tokens,
                    "thinking": { "type": "disabled" },
                    "output_config": { "effort": "medium" },
                    "stream": stream,
                    "system": system,
                    "messages": ask.messages,
                })
            }
            // `max_tokens` rather than `max_completion_tokens`: the newer name is
            // OpenAI's own and the older one is what every other service
            // answering this shape accepts.
            Kind::OpenAi => {
                let mut messages = vec![json!({ "role": "system", "content": ask.system })];
                messages.extend(ask.messages.iter().cloned());
                json!({
                    "model": model,
                    "max_tokens": ask.max_tokens,
                    "stream": stream,
                    "messages": messages,
                })
            }
        }
    }

    /// The in-flight request, owning everything it needs.
    ///
    /// `use<>`: without it the future inherits the arguments' lifetimes and
    /// cannot outlive the handler that built it, which is exactly what streaming
    /// a response out of one requires.
    fn send(
        &self,
        http: &reqwest::Client,
        body: Value,
    ) -> impl Future<Output = reqwest::Result<reqwest::Response>> + Send + 'static + use<> {
        let req = http.post(self.endpoint()).json(&body);
        let req = match self.kind {
            Kind::Anthropic => req
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01"),
            Kind::OpenAi => req.bearer_auth(&self.api_key),
        };
        req.send()
    }

    /// The whole answer, awaited.
    pub async fn complete(&self, http: &reqwest::Client, ask: &Ask<'_>) -> Result<String, Error> {
        if self.api_key.trim().is_empty() {
            return Err(Error::Unavailable("no API key is set".into()));
        }
        let resp = self
            .send(http, self.body(ask, false))
            .await
            .map_err(|e| Error::Failed(format!("{} request failed: {e}", self.kind.as_str())))?;

        let status = resp.status();
        let json: Value = resp.json().await.map_err(|e| {
            Error::Failed(format!("{} response unparseable: {e}", self.kind.as_str()))
        })?;
        if !status.is_success() {
            return Err(Error::Failed(format!(
                "{} returned {status}: {}",
                self.kind.as_str(),
                error_message(&json)
            )));
        }
        if cut_off(self.kind, &json) {
            return Err(Error::Failed(format!(
                "{} stopped at max_tokens before finishing",
                self.kind.as_str()
            )));
        }
        self.text(&json)
    }

    /// Whether the endpoint answers this key with usable text.
    ///
    /// A full request rather than a cheap one, because a reasoning model spends
    /// the token budget before it writes anything: a request too small to finish
    /// returns success and no text, which would report a configuration that
    /// cannot answer as a working one.
    pub async fn probe(&self, http: &reqwest::Client, ask: &Ask<'_>) -> Result<(), Error> {
        self.complete(http, ask).await.map(|_| ())
    }

    /// The answer a piece at a time.
    ///
    /// Streamed rather than awaited whole because the reader is mid-line while
    /// it arrives: the model is what it is, so the only lever on how long this
    /// *feels* is when the first words land.
    pub fn stream(
        &self,
        http: &reqwest::Client,
        ask: &Ask<'_>,
    ) -> impl Stream<Item = Result<String, Error>> + Send + 'static + use<> {
        let kind = self.kind;
        let missing = self.api_key.trim().is_empty();
        let request = self.send(http, self.body(ask, true));
        async_stream::stream! {
            if missing {
                yield Err(Error::Unavailable("no API key is set".into()));
                return;
            }
            let resp = match request.await {
                Ok(r) => r,
                Err(e) => {
                    yield Err(Error::Failed(format!("{} request failed: {e}", kind.as_str())));
                    return;
                }
            };

            let status = resp.status();
            if !status.is_success() {
                // An error answers as one JSON body, not as an event stream.
                let json: Value = resp.json().await.unwrap_or_default();
                yield Err(Error::Failed(format!(
                    "{} returned {status}: {}", kind.as_str(), error_message(&json)
                )));
                return;
            }

            let mut body = resp.bytes_stream();
            // Chunks split anywhere, including mid-line and mid-UTF-8, so frames
            // are reassembled here rather than decoded per chunk.
            let mut buf: Vec<u8> = Vec::new();
            let mut any_text = false;
            let mut ran_out = false;
            while let Some(chunk) = body.next().await {
                match chunk {
                    Ok(bytes) => buf.extend_from_slice(&bytes),
                    Err(e) => {
                        yield Err(Error::Failed(format!("{} stream broke: {e}", kind.as_str())));
                        return;
                    }
                }
                while let Some(nl) = buf.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=nl).collect();
                    let Some(frame) = frame(kind, &line) else { continue };
                    ran_out |= frame.cut_off;
                    if !frame.text.is_empty() {
                        any_text = true;
                        yield Ok(frame.text);
                    }
                }
            }
            // A stream that ended having said nothing is a failure, not an empty
            // answer: the surface waiting on it would otherwise sit on its
            // placeholder with nothing to show and no reason why.
            if !any_text {
                yield Err(Error::Failed(if ran_out {
                    format!("{} stopped at max_tokens before writing any text", kind.as_str())
                } else {
                    format!("no text in the {} reply", kind.as_str())
                }));
            }
        }
    }

    /// The text of a completed response.
    ///
    /// Anthropic's first block can be a `thinking` block on a thinking-capable
    /// model, so the first *text* block is taken rather than the first block.
    fn text(&self, json: &Value) -> Result<String, Error> {
        let found = match self.kind {
            Kind::Anthropic => json["content"]
                .as_array()
                .and_then(|blocks| blocks.iter().find(|b| b["type"] == "text"))
                .and_then(|block| block["text"].as_str()),
            Kind::OpenAi => json["choices"][0]["message"]["content"].as_str(),
        };
        // An empty string is a failure, not an answer. Both shapes return one
        // when the reply was all reasoning, and passing it on as success leaves
        // a caller writing a blank where it meant to write a gloss.
        found
            .map(str::to_string)
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| Error::Failed(format!("no text in the {} reply", self.kind.as_str())))
    }
}

/// Both shapes report an error the same way.
fn error_message(json: &Value) -> String {
    json["error"]["message"]
        .as_str()
        .unwrap_or("unknown API error")
        .to_string()
}

/// Whether a completed reply was cut off at `max_tokens` rather than finished.
///
/// A reasoning model can spend the whole budget thinking and return no text at
/// all, so this separates a request that was too small from a model with
/// nothing to say.
fn cut_off(kind: Kind, json: &Value) -> bool {
    match kind {
        Kind::Anthropic => json["stop_reason"] == "max_tokens",
        Kind::OpenAi => json["choices"][0]["finish_reason"] == "length",
    }
}

/// What one SSE line carries.
struct Frame {
    /// Empty for a frame carrying no text — the OpenAI shape opens with a
    /// role-only delta, and sends empty content while it is still reasoning.
    text: String,
    cut_off: bool,
}

/// The frame one SSE line carries, or nothing — every other line is an event
/// name, a blank separator, the `[DONE]` sentinel, or a frame about something
/// else (usage, block boundaries).
fn frame(kind: Kind, line: &[u8]) -> Option<Frame> {
    let line = std::str::from_utf8(line).ok()?.trim();
    let payload = line.strip_prefix("data:")?.trim();
    if payload == "[DONE]" {
        return None;
    }
    let json: Value = serde_json::from_str(payload).ok()?;
    let (text, cut_off) = match kind {
        Kind::Anthropic => {
            let text = if json["type"] == "content_block_delta"
                && json["delta"]["type"] == "text_delta"
            {
                json["delta"]["text"].as_str().unwrap_or_default()
            } else {
                ""
            };
            (text, json["delta"]["stop_reason"] == "max_tokens")
        }
        Kind::OpenAi => (
            json["choices"][0]["delta"]["content"]
                .as_str()
                .unwrap_or_default(),
            json["choices"][0]["finish_reason"] == "length",
        ),
    };
    Some(Frame {
        text: text.to_string(),
        cut_off,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(kind: Kind, base_url: &str, model: &str) -> Provider {
        Provider {
            kind,
            base_url: base_url.into(),
            model: model.into(),
            api_key: "k".into(),
        }
    }

    fn ask() -> Ask<'static> {
        Ask {
            system: "be brief",
            messages: &[],
            max_tokens: 10,
            default_model: "built-in",
            cache_system: false,
        }
    }

    #[test]
    fn endpoint_falls_back_to_the_kinds_own_base() {
        assert_eq!(
            provider(Kind::Anthropic, "", "").endpoint(),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            provider(Kind::OpenAi, "", "").endpoint(),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn a_configured_base_url_keeps_its_own_version_segment() {
        assert_eq!(
            provider(Kind::OpenAi, "https://openrouter.ai/api/v1/", "").endpoint(),
            "https://openrouter.ai/api/v1/chat/completions"
        );
    }

    #[test]
    fn the_prompts_model_is_used_only_when_the_provider_names_none() {
        assert_eq!(provider(Kind::OpenAi, "", "").model_for(&ask()), "built-in");
        assert_eq!(
            provider(Kind::OpenAi, "", " gpt-x ").model_for(&ask()),
            "gpt-x"
        );
    }

    #[test]
    fn the_system_prompt_becomes_a_message_in_the_openai_shape() {
        let body = provider(Kind::OpenAi, "", "").body(&ask(), false);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "be brief");
        assert!(body["system"].is_null());
    }

    #[test]
    fn caching_is_asked_for_only_where_it_is_priced() {
        let mut a = ask();
        a.cache_system = true;
        let anthropic = provider(Kind::Anthropic, "", "").body(&a, false);
        assert_eq!(
            anthropic["system"][0]["cache_control"]["type"], "ephemeral",
            "the Anthropic shape carries the cache marker"
        );
        let openai = provider(Kind::OpenAi, "", "").body(&a, false);
        assert!(
            openai["messages"][0].get("cache_control").is_none(),
            "the OpenAI shape has no field for it"
        );
    }

    fn text_of(kind: Kind, line: &[u8]) -> Option<String> {
        frame(kind, line).map(|f| f.text).filter(|t| !t.is_empty())
    }

    #[test]
    fn deltas_are_read_out_of_both_stream_shapes() {
        let anthropic =
            br#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"hi"}}"#;
        assert_eq!(text_of(Kind::Anthropic, anthropic).as_deref(), Some("hi"));
        let openai = br#"data: {"choices":[{"delta":{"content":"hi"}}]}"#;
        assert_eq!(text_of(Kind::OpenAi, openai).as_deref(), Some("hi"));
    }

    #[test]
    fn frames_that_are_not_text_carry_none() {
        assert!(frame(Kind::OpenAi, b"data: [DONE]").is_none());
        assert!(frame(Kind::Anthropic, b"event: message_start").is_none());
        assert!(frame(Kind::Anthropic, b"\n").is_none());
        // The OpenAI shape opens with a role-only delta carrying no content, and
        // sends empty content for as long as it is still reasoning.
        assert!(
            text_of(
                Kind::OpenAi,
                br#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#
            )
            .is_none()
        );
        assert!(
            text_of(
                Kind::OpenAi,
                br#"data: {"choices":[{"delta":{"content":"","reasoning_content":"hm"}}]}"#
            )
            .is_none()
        );
    }

    #[test]
    fn running_out_of_budget_is_visible_in_both_stream_shapes() {
        let openai = br#"data: {"choices":[{"delta":{},"finish_reason":"length"}]}"#;
        assert!(frame(Kind::OpenAi, openai).unwrap().cut_off);
        let anthropic = br#"data: {"type":"message_delta","delta":{"stop_reason":"max_tokens"}}"#;
        assert!(frame(Kind::Anthropic, anthropic).unwrap().cut_off);
    }

    #[test]
    fn a_reply_that_is_all_reasoning_is_an_error() {
        let openai = json!({ "choices": [{ "message": { "content": "" } }] });
        assert!(provider(Kind::OpenAi, "", "").text(&openai).is_err());
        let anthropic = json!({ "content": [{ "type": "thinking", "thinking": "hm" }] });
        assert!(provider(Kind::Anthropic, "", "").text(&anthropic).is_err());
    }

    #[test]
    fn a_completed_reply_reports_running_out_of_budget() {
        assert!(cut_off(
            Kind::OpenAi,
            &json!({ "choices": [{ "finish_reason": "length" }] })
        ));
        assert!(cut_off(
            Kind::Anthropic,
            &json!({ "stop_reason": "max_tokens" })
        ));
        assert!(!cut_off(
            Kind::OpenAi,
            &json!({ "choices": [{ "finish_reason": "stop" }] })
        ));
    }

    #[test]
    fn a_leading_thinking_block_is_skipped() {
        let json = json!({
            "content": [
                { "type": "thinking", "thinking": "" },
                { "type": "text", "text": "the answer" },
            ]
        });
        assert_eq!(
            provider(Kind::Anthropic, "", "").text(&json).unwrap(),
            "the answer"
        );
    }

    #[test]
    fn kind_round_trips_through_its_name() {
        for kind in [Kind::Anthropic, Kind::OpenAi] {
            assert_eq!(Kind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(Kind::parse("gemini"), None);
    }
}
