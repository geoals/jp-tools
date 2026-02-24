use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("LLM request failed: {0}")]
    Failed(String),
}

#[cfg_attr(test, mockall::automock)]
pub trait LlmDefiner: Send + Sync {
    fn define(
        &self,
        word: &str,
        sentence_context: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, LlmError>> + Send>>;
}

pub struct AnthropicDefiner {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

impl AnthropicDefiner {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            model,
        }
    }

    fn build_request_body(&self, word: &str, sentence_context: &str) -> Value {
        serde_json::json!({
            "model": self.model,
            "max_tokens": 512,
            "system": SYSTEM_PROMPT,
            "messages": [
                {
                    "role": "user",
                    "content": format!(
                        "Word: {word}\nSentence: {sentence_context}"
                    )
                }
            ]
        })
    }
}

const SYSTEM_PROMPT: &str = "\
You are a Japanese language expert. Given a Japanese word and the sentence it appeared in, \
explain in English what concept or nuance the word expresses — do not simply translate it.\n\n\
Then state how common the word is among native Japanese adults. Use one of these labels:\n\
- CORE: every adult knows and uses this regularly\n\
- COMMON: widely known, most adults use it\n\
- LITERATE: educated adults know it; may be literary, formal, or domain-specific\n\
- RARE: many native speakers would not recognize this\n\n\
Keep your response to 2-4 sentences. Do not use markdown formatting.";

impl LlmDefiner for AnthropicDefiner {
    fn define(
        &self,
        word: &str,
        sentence_context: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, LlmError>> + Send>> {
        let body = self.build_request_body(word, sentence_context);
        let client = self.client.clone();
        let api_key = self.api_key.clone();

        Box::pin(async move {
            let response = client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| LlmError::Failed(e.to_string()))?;

            let status = response.status();
            let json: Value = response
                .json()
                .await
                .map_err(|e| LlmError::Failed(format!("failed to parse response: {e}")))?;

            if !status.is_success() {
                let msg = json["error"]["message"]
                    .as_str()
                    .unwrap_or("unknown API error");
                return Err(LlmError::Failed(format!("API returned {status}: {msg}")));
            }

            extract_text(&json)
        })
    }
}

/// Extract the text content from an Anthropic Messages API response.
fn extract_text(json: &Value) -> Result<String, LlmError> {
    json["content"]
        .as_array()
        .and_then(|blocks| blocks.first())
        .and_then(|block| block["text"].as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| LlmError::Failed("no text content in response".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_text_from_valid_response() {
        let json = serde_json::json!({
            "content": [
                {
                    "type": "text",
                    "text": "This is the definition."
                }
            ]
        });
        assert_eq!(extract_text(&json).unwrap(), "This is the definition.");
    }

    #[test]
    fn extract_text_empty_content_array() {
        let json = serde_json::json!({ "content": [] });
        assert!(extract_text(&json).is_err());
    }

    #[test]
    fn extract_text_missing_content_field() {
        let json = serde_json::json!({ "id": "msg_123" });
        assert!(extract_text(&json).is_err());
    }

    #[test]
    fn extract_text_missing_text_field_in_block() {
        let json = serde_json::json!({
            "content": [{ "type": "text" }]
        });
        assert!(extract_text(&json).is_err());
    }

    #[test]
    fn build_request_body_structure() {
        let definer = AnthropicDefiner::new("test-key".into(), "claude-sonnet-4-6".into());
        let body = definer.build_request_body("食べる", "毎日ラーメンを食べる");

        assert_eq!(body["model"], "claude-sonnet-4-6");
        assert_eq!(body["max_tokens"], 512);
        assert!(body["system"].as_str().unwrap().contains("Japanese"));

        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        let content = messages[0]["content"].as_str().unwrap();
        assert!(content.contains("食べる"));
        assert!(content.contains("毎日ラーメンを食べる"));
    }

    #[tokio::test]
    #[ignore = "requires ANTHROPIC_API_KEY environment variable"]
    async fn anthropic_definer_integration() {
        let api_key = std::env::var("ANTHROPIC_API_KEY").expect("set ANTHROPIC_API_KEY");
        let definer = AnthropicDefiner::new(api_key, "claude-sonnet-4-6".into());
        let result = definer
            .define("食べる", "毎日ラーメンを食べる")
            .await
            .unwrap();
        assert!(!result.is_empty());
        println!("LLM definition: {result}");
    }
}
