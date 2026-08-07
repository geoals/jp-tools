//! The process-wide [`Highlighter`], and nothing else.
//!
//! The pipeline itself — what a word is worth knowing about, where it sits in
//! the line, and the seven inputs the tokenizer takes — is
//! [`jp_core::highlight`], because it is the join of `tokenize` and `knowledge`
//! and yt-mine asks it the same questions. What stays here is the caching.

pub use jp_core::highlight::{Analyzed, Highlighter, Span, analyze, spans};

/// Built on first use rather than at startup — the dictionary load is seconds
/// of CPU and `#read` is one tab of six.
///
/// Never rebuilt, which is the limitation: importing a dictionary changes the
/// tints only after a restart. Ingest builds its own each pass, so nothing
/// *stored* goes stale.
pub type Shared = std::sync::Arc<tokio::sync::OnceCell<std::sync::Arc<Highlighter>>>;

/// The shared highlighter, building it if this is the first line to need it.
///
/// `None` when it could not be built (a missing or unreadable Sudachi
/// dictionary); the reader then streams untinted. A failure is not memoized.
pub async fn shared(state: &crate::app::AppState) -> Option<std::sync::Arc<Highlighter>> {
    let cell = state.highlighter.clone();
    let built = cell
        .get_or_try_init(|| async {
            Highlighter::build(&state.knowledge, &state.sudachi_dict_path)
                .await
                .map(std::sync::Arc::new)
        })
        .await;
    match built {
        Ok(h) => Some(h.clone()),
        Err(e) => {
            tracing::warn!(error = %e, "reader highlighter unavailable — streaming untinted");
            None
        }
    }
}
