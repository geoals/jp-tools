//! Building this app's [`Highlighter`], and nothing else.
//!
//! The pipeline itself — what a word is worth knowing about, and where it sits
//! in the line — is [`jp_core::highlight`], because it is the join of `tokenize`
//! and `knowledge` and yt-mine asks it the same questions. What stays here is
//! the *six inputs*: they come out of `ingest`, against this app's databases.

pub use jp_core::highlight::{Analyzed, Highlighter, Span, analyze, spans};

use jp_core::tokenize::{MasterWords, SudachiTokenizer};

/// The process-wide [`Highlighter`], built on first use rather than at startup —
/// the dictionary load is seconds of CPU and `#read` is one tab of six.
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
    let built: Result<&std::sync::Arc<Highlighter>, crate::error::AppError> = cell
        .get_or_try_init(|| async {
            let dict_path = state.sudachi_dict_path.clone();
            let vocab = crate::ingest::mined_vocab(state).await?;
            let lexicon = crate::ingest::master_lexicon(state).await?;
            let readings = crate::ingest::master_readings(state).await?;
            // The same six inputs the ingest pass builds its tokenizer with,
            // and they have to stay the same six. Without the frequency ranks
            // a word written in kana whose reading names several master
            // headwords is left as the kana — うかがう rather than 窺う, which
            // no dictionary lists as a headword — so the wordhood gate calls it
            // a non-word and the reader tints nothing. Ingest meanwhile files
            // the same token under 窺う and counts it. Two pipelines, two
            // answers, and the tinted one was the wrong one.
            let ranks = crate::ingest::frequency_ranks(state, &readings).await?;
            let preferred = crate::ingest::preferred_readings(state).await?;
            let conjugatable = crate::ingest::conjugatable(state).await?;
            let standard = crate::ingest::standard_readings(state).await?;
            let master = MasterWords::new(lexicon.clone(), &readings);
            let headwords: Vec<String> = lexicon.iter().cloned().collect();
            let word_ranks = crate::ingest::all_frequency_ranks(state, &headwords).await?;
            // Dictionary load is CPU-bound and measured in seconds; it must not
            // sit on the runtime while other readers' streams are polling.
            tokio::task::spawn_blocking(move || {
                let tokenizer = SudachiTokenizer::new(&dict_path, vocab)
                    .map_err(|e| crate::error::AppError::Upstream(format!("sudachi: {e}")))?
                    .with_lexicon(lexicon.clone())
                    .with_master_readings(&readings)
                    .with_frequency(ranks)
                    .with_preferred_readings(preferred)
                    .with_conjugatable(conjugatable)
                    .with_standard(&standard);
                Ok(std::sync::Arc::new(Highlighter::new(
                    tokenizer, lexicon, master, word_ranks,
                )))
            })
            .await
            .map_err(|e| {
                crate::error::AppError::Upstream(format!("highlighter build panicked: {e}"))
            })?
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
