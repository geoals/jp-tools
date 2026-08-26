//! The CompactDef gloss for a mined card, behind a trait so the fake and the
//! route tests can stand in for it.
//!
//! The prompt is [`jp_mine_core::compactdef`], shared with kotodex-server. It used
//! to be a second one written here, on a four-tier familiarity scale that
//! predated the sharpened rubric and carried no FLAVOR axis at all — precisely
//! the drift `jp_mine_core::tags` exists to prevent.

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
    api_key: String,
}

impl CompactDefiner {
    pub fn new(api_key: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
        }
    }
}

impl LlmDefiner for CompactDefiner {
    fn define(
        &self,
        word: &str,
        sentence_context: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, LlmError>> + Send>> {
        let (client, api_key) = (self.client.clone(), self.api_key.clone());
        let (word, sentence) = (word.to_string(), sentence_context.to_string());
        Box::pin(async move {
            jp_mine_core::compactdef::compact_def(&client, &api_key, &word, &sentence)
                .await
                .map_err(|e| LlmError::Failed(e.to_string()))
        })
    }
}
