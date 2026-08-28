//! The reader's "explain this line" prompt, and which model answers it.
//!
//! The request shapes and the streaming live in [`jp_mine_core::llm`], shared
//! with the card gloss — what stays here is the prompt, and the resolution of
//! *which* provider a given install means.

use std::sync::LazyLock;

use jp_mine_core::llm::{Ask, Kind, Provider};

use crate::app::AppState;
use crate::db;
use crate::error::AppError;

/// Built once from the shared tag rubric (`jp_mine_core::tags`) plus the
/// explain-path framing. The FAMILIARITY/FLAVOR definitions are the same source
/// of truth `compactdef.rs` uses, so the two paths tag identically.
static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    use jp_mine_core::tags::{FAMILIARITY_RUBRIC, FLAVOR_RUBRIC};
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

/// The model this prompt was tuned against, used unless the reader has named one.
///
/// The explain button is a short interactive lookup read once and thrown away,
/// so it does not need the model the cards are written with. The tags it prints
/// therefore come off a different model than `compactdef`'s, which is why the
/// rubric is shared source and not a paraphrase: the wording is the only thing
/// holding the two axes together.
const MODEL: &str = "claude-sonnet-5";

/// The cap is a cost bound, not a target: an answer this prompt asks to be "very
/// concise" runs well under it, and hitting it truncates mid-sentence.
const MAX_TOKENS: u32 = 400;

/// Which model this install asks, or `None` when no key has been given.
///
/// Resolved per call rather than at boot, because the whole point of the key
/// being a setting is that pasting one starts working without a restart.
/// `KOTODEX_ANTHROPIC_API_KEY` stays as the fallback — `setup.sh` writes it, so
/// every install that predates the settings row still has its key there.
pub async fn provider(state: &AppState) -> Result<Option<Provider>, AppError> {
    let settings = db::load_settings(&state.local).await?;
    let stored = db::llm_api_key(&state.local).await?;
    let Some(api_key) = stored.or_else(|| state.env_api_key.clone()) else {
        return Ok(None);
    };
    Ok(Some(Provider {
        // An unparseable name means a row written by hand; the default shape is a
        // better answer than refusing to explain anything.
        kind: Kind::parse(&settings.llm_provider).unwrap_or(Kind::Anthropic),
        base_url: settings.llm_base_url,
        model: settings.llm_model,
        api_key,
    }))
}

/// Whether a key is configured at all. Says nothing about whether it works —
/// [`check`] is what asks.
pub async fn available(state: &AppState) -> bool {
    provider(state).await.ok().flatten().is_some()
}

/// Send the cheapest possible request and report which model answered.
///
/// One token and a one-word prompt: this is asked when a key is pasted, and the
/// question is only whether the endpoint accepts it.
pub async fn check(state: &AppState) -> Result<String, AppError> {
    let Some(provider) = provider(state).await? else {
        return Err(AppError::BadRequest("no API key is set".into()));
    };
    let messages = vec![serde_json::json!({ "role": "user", "content": "hi" })];
    provider
        .probe(
            &state.http,
            &Ask {
                system: "Reply with one word.",
                messages: &messages,
                max_tokens: 1,
                default_model: MODEL,
                cache_system: false,
            },
        )
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?;
    Ok(if provider.model.trim().is_empty() {
        MODEL.to_string()
    } else {
        provider.model.trim().to_string()
    })
}

/// Ask the model to explain `context`'s last line, a piece at a time.
pub fn explain_stream(
    http: &reqwest::Client,
    provider: &Provider,
    context: &[String],
    focus: &str,
) -> impl futures_util::Stream<Item = Result<String, AppError>> + Send + 'static + use<> {
    let messages = vec![serde_json::json!({
        "role": "user",
        "content": user_message(context, focus),
    })];
    let stream = provider.stream(
        http,
        &Ask {
            system: SYSTEM_PROMPT.as_str(),
            messages: &messages,
            max_tokens: MAX_TOKENS,
            default_model: MODEL,
            // One line and a few of context, different every time: there is no
            // repeated prefix worth the cache write.
            cache_system: false,
        },
    );
    futures_util::StreamExt::map(stream, |chunk| {
        chunk.map_err(|e| AppError::Upstream(e.to_string()))
    })
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_line_is_the_one_to_explain() {
        let context = ["先を歩く".to_string(), "振り返った".to_string()];
        let msg = user_message(&context, "");
        assert!(msg.contains("Context (earlier lines, oldest first):\n先を歩く"));
        assert!(msg.ends_with("Line to explain:\n振り返った"));
    }

    #[test]
    fn a_single_line_carries_no_context_block() {
        let msg = user_message(&["振り返った".to_string()], "振り返る");
        assert!(!msg.contains("Context"));
        assert!(msg.ends_with("Focus word: 振り返る"));
    }
}
