//! The CompactDef gloss for a mined card, behind a trait so the fake and the
//! route tests can stand in for it.
//!
//! The prompt is [`jp_mine_core::compactdef`], shared with kotodex-server, so a
//! transcript card and a VN card are glossed by one rubric.

use std::future::Future;
use std::pin::Pin;

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("LLM request failed: {0}")]
    Failed(String),
}

#[cfg_attr(test, mockall::automock)]
pub trait LlmDefiner: Send + Sync {
    /// `word` is the surface as the transcript spelt it, and `sentence` carries
    /// that span in `<b>` tags — the gloss is rated on the spelling the reader
    /// met, never on the headword.
    fn define(
        &self,
        word: &str,
        sentence_context: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, LlmError>> + Send>>;
}

pub struct CompactDefiner {
    client: reqwest::Client,
    provider: jp_mine_core::llm::Provider,
}

impl CompactDefiner {
    /// Anthropic behind the API key, which is all yt-mine has ever configured.
    /// The provider is a setting in kotodex-server because a reader sets it there;
    /// this tool has no such surface.
    pub fn new(api_key: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            provider: jp_mine_core::llm::Provider {
                kind: jp_mine_core::llm::Kind::Anthropic,
                base_url: String::new(),
                model: String::new(),
                api_key,
            },
        }
    }
}

impl LlmDefiner for CompactDefiner {
    fn define(
        &self,
        word: &str,
        sentence_context: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, LlmError>> + Send>> {
        let (client, provider) = (self.client.clone(), self.provider.clone());
        let (word, sentence) = (word.to_string(), sentence_context.to_string());
        Box::pin(async move {
            jp_mine_core::compactdef::compact_def(&client, &provider, &word, &sentence)
                .await
                .map_err(|e| LlmError::Failed(e.to_string()))
        })
    }
}
