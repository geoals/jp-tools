//! A one-shot model call that writes an ultra-short "CompactDef" gloss for a
//! mined card — the sense the target word carries in its sentence, compressed to
//! something readable in under 2 seconds (~8 Japanese characters).
//!
//! Shared by every surface that mines a card, because the gloss is a property
//! of the card and not of where it came from: kotodex-server writes one after
//! Yomitan or the overlay adds a note, and yt-mine writes one on export.

use std::sync::LazyLock;

use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum CompactDefError {
    #[error("CompactDef failed: {0}")]
    Failed(String),
    /// The backend itself is not answering — the CLI would not start, exited
    /// non-zero, or printed nothing. Separate from `Failed` because it is not
    /// about this card: a caller looping over thousands of them must stop, not
    /// spawn another few hundred processes into a usage limit.
    #[error("CompactDef backend unavailable: {0}")]
    Unavailable(String),
}

impl From<crate::llm::Error> for CompactDefError {
    fn from(e: crate::llm::Error) -> Self {
        match e {
            crate::llm::Error::Failed(m) => CompactDefError::Failed(m),
            crate::llm::Error::Unavailable(m) => CompactDefError::Unavailable(m),
        }
    }
}

use crate::llm::{Ask, Provider};
use crate::tags::{FAMILIARITY_RUBRIC, FLAVOR_RUBRIC, TagLine};

/// The model this prompt was tuned against, used unless the reader has named one
/// (`llm::Provider::model`). The tag-axis experiment found no thinking and no
/// external frequency signals to be best, and that request shape carries over
/// unchanged. Opus is preferred over Sonnet for the meaning/usage prose.
const MODEL: &str = "claude-opus-5";

/// Built once from the shared tag rubric ([`crate::tags`]) plus the CompactDef-
/// specific framing and output format. The FAMILIARITY/FLAVOR definitions live
/// in `tags.rs` so this and kotodex-server's explain prompt can never drift apart
/// again — and so yt-mine cannot grow a third paraphrase of them, which is
/// exactly what its old `LlmDefiner` was.
static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    format!(
        "\
You write a compact ENGLISH gloss (\"CompactDef\") for a Japanese vocab \
flashcard. It sits at the top of the card back as a fast recognition check; the \
full Japanese dictionary entry is shown below it. The learner is a native \
English speaker. Gloss the sense the word carries IN THE GIVEN SENTENCE.\n\n\
You are given the target as it is WRITTEN in the sentence — conjugated, and in \
whatever script the author used — and marked with <b> tags in the sentence \
itself. No dictionary headword is supplied, deliberately. Work out which word it \
is from the sentence; gloss that word's sense, but rate FAMILIARITY on the \
written form you were given.\n\n\
Output exactly two lines and nothing else — no preamble, no markdown, and never \
an XML or HTML tag or label of your own:\n\
- Line 1 — the meaning, optionally followed by \". \" and one short usage note.\n\
- Line 2 — [FAMILIARITY · ]FLAVOR[ · FLAVOR2[ · FLAVOR3]][ (structural)]\n\
Line 2 is machine-read. Every tag is separated by \" · \". A baseline formality \
is always present, even when a mark is more informative. FAMILIARITY is not: \
most lines start with the baseline.\n\n\
MEANING/USAGE: nuance-carrying English. A bare one/two-word translation ONLY for \
a concrete 1-to-1 term (焼却炉 → incinerator); otherwise a short phrase that \
carries the actual nuance. Optionally one short usage note — a fixed collocation, \
a polarity restriction, or the typical speaker — where citing the Japanese word \
or its usual phrase is fine. Any Japanese reading you cite: hiragana, never \
romaji.\n\n\
{FAMILIARITY_RUBRIC}\n\n\
{FLAVOR_RUBRIC}\n\n\
STRUCTURAL (optional trailing parenthetical, orthogonal): (idiom) (mimetic) \
(fixed phrase) (proverb) (name) (four-char idiom).\n\n\
Judge from the word, the sentence, and your own knowledge ALONE. No preamble, \
no markdown."
    )
});

/// The exact system prompt sent with every CompactDef call. Exposed because
/// tuning the rubric means reading what is actually being asked, and a
/// paraphrase of it is how the two callers drifted apart in the first place.
pub fn system_prompt() -> &'static str {
    &SYSTEM_PROMPT
}

/// Generate the CompactDef gloss for `target` as used in `sentence`.
///
/// `target` is the surface form — the spelling the page used, not the
/// dictionary headword. The headword is withheld on purpose: shown 饐える, the
/// model prices the kanji and tags a phrase people say as RARE. The sentence
/// keeps its `<b>` markers around the target and nothing else, so the model can
/// find the span when the surface is short or repeated.
pub async fn compact_def(
    http: &reqwest::Client,
    provider: &Provider,
    target: &str,
    sentence: &str,
) -> Result<String, CompactDefError> {
    let mut messages = vec![serde_json::json!({
        "role": "user",
        "content": format!("Sentence: {sentence}\nTarget: {target}"),
    })];

    // One correction round. The tag line is the machine-readable half of the
    // field, so a malformed one is sent back rather than written: the repairable
    // shapes are already repaired by `TagLine::parse`, which leaves only a
    // missing baseline or an invented tag — both of them judgements the model
    // has to make again, not formatting this side can guess at.
    for attempt in 0..2 {
        let raw = clean_gloss(&request(http, provider, &messages).await?);
        match canonical_gloss(&raw) {
            Ok(gloss) => return Ok(gloss),
            Err(why) if attempt == 0 => {
                messages.push(serde_json::json!({ "role": "assistant", "content": raw }));
                messages.push(serde_json::json!({
                    "role": "user",
                    "content": format!(
                        "Line 2 is invalid: {why}. Send both lines again, corrected."
                    ),
                }));
            }
            Err(why) => return Err(CompactDefError::Failed(format!("bad tag line: {why}"))),
        }
    }
    unreachable!("the loop returns on both branches of its last iteration")
}

/// Run one `claude -p` call and return its stdout.
fn run_cli(system: &str, message: &str) -> Result<String, CompactDefError> {
    let out = std::process::Command::new("claude")
        .args(["-p", "--model", "opus", "--effort", "low"])
        .args(["--setting-sources", ""])
        // Writing a gloss needs no tools, and their definitions are 3,800 of
        // the ~12,000 tokens the CLI sends — a third of the bill over a run of
        // thousands. Naming them is unfortunate but there is no "no tools"
        // switch; a tool missing from this list costs tokens, not correctness.
        .arg("--disallowed-tools")
        .args([
            "Bash",
            "Read",
            "Write",
            "Edit",
            "Glob",
            "Grep",
            "WebFetch",
            "WebSearch",
            "Task",
            "TodoWrite",
            "NotebookEdit",
        ])
        .args(["--system-prompt", system])
        .arg(message)
        // Nothing here should read the filesystem, and a cwd with a CLAUDE.md
        // in it is exactly what --setting-sources is shutting out.
        .current_dir(std::env::temp_dir())
        .output()
        .map_err(|e| CompactDefError::Unavailable(format!("could not run `claude`: {e}")))?;

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() || stdout.trim().is_empty() {
        let err = String::from_utf8_lossy(&out.stderr);
        let why = if err.trim().is_empty() {
            "no output".to_string()
        } else {
            err.trim().to_string()
        };
        return Err(CompactDefError::Unavailable(format!("claude CLI: {why}")));
    }
    Ok(stdout)
}

/// The batch system prompt: the same rubric, asked for many words at once.
static BATCH_PROMPT: LazyLock<String> = LazyLock::new(|| {
    format!(
        "{}\n\n\
BATCH MODE: the user sends numbered items, each `N. Sentence: … | Target: …`. \
Answer every item, one output line each, in order:\n\
N. <meaning line> ## <tag line>\n\
Nothing else — no blank lines between items, no preamble, no markdown. The \
two lines of the normal format become the two halves of one line, split by \
\" ## \". Never use \" ## \" inside the meaning.",
        system_prompt()
    )
});

/// Gloss many cards in one CLI call.
///
/// One round trip per card is the wrong shape for a re-tag of thousands: the
/// system prompt is ~1,300 tokens and was being resent every time, and each
/// call is a process spawn plus a cold model. Batching amortises the prompt
/// over the whole batch and cuts wall-clock time by roughly the batch size.
///
/// Returns one result per input, in order. An item the model skipped or
/// answered unparseably comes back `Err(Failed)` and the rest still land; a
/// backend failure is `Unavailable` for every item, so the caller can stop.
pub fn compact_def_batch_cli(items: &[(String, String)]) -> Vec<Result<String, CompactDefError>> {
    let message = items
        .iter()
        .enumerate()
        .map(|(i, (target, sentence))| {
            format!("{}. Sentence: {sentence} | Target: {target}", i + 1)
        })
        .collect::<Vec<_>>()
        .join("\n");

    let stdout = match run_cli(&BATCH_PROMPT, &message) {
        Ok(s) => s,
        Err(e) => {
            let why = e.to_string();
            return items
                .iter()
                .map(|_| Err(CompactDefError::Unavailable(why.clone())))
                .collect();
        }
    };

    let answers = parse_batch(&stdout, items.len());
    answers
        .into_iter()
        .map(|a| {
            a.ok_or_else(|| CompactDefError::Failed("no answer in the batch reply".into()))
                .and_then(|raw| {
                    canonical_gloss(&raw)
                        .map_err(|why| CompactDefError::Failed(format!("bad tag line: {why}")))
                })
        })
        .collect()
}

/// Pull `N. meaning ## tags` lines out of a batch reply, indexed by the model's
/// own numbering rather than by position — a skipped or duplicated item must
/// not shift every answer after it onto the wrong card.
fn parse_batch(stdout: &str, len: usize) -> Vec<Option<String>> {
    let mut answers = vec![None; len];
    for line in stdout.lines() {
        let line = line.trim();
        let Some((index, rest)) = line.split_once('.') else {
            continue;
        };
        let Ok(index) = index.trim().parse::<usize>() else {
            continue;
        };
        let Some((meaning, tags)) = rest.split_once("##") else {
            continue;
        };
        if index == 0 || index > len {
            continue;
        }
        let meaning = clean_gloss(meaning);
        answers[index - 1] = Some(format!("{meaning}<br>{}", tags.trim()));
    }
    answers
}

/// Split a cleaned gloss into its meaning and tag halves and re-render the tag
/// line in canonical form. The meaning line is passed through untouched.
fn canonical_gloss(gloss: &str) -> Result<String, String> {
    let (meaning, tags) = gloss
        .rsplit_once("<br>")
        .ok_or("no tag line — expected a meaning line and a tag line")?;
    let tags = TagLine::parse(tags).map_err(|e| e.to_string())?;
    Ok(format!("{meaning}<br>{tags}"))
}

async fn request(
    http: &reqwest::Client,
    provider: &Provider,
    messages: &[Value],
) -> Result<String, CompactDefError> {
    provider
        .complete(
            http,
            &Ask {
                system: SYSTEM_PROMPT.as_str(),
                messages,
                max_tokens: 300,
                default_model: MODEL,
                // The system block is the same ~1,300 tokens on every card and is
                // most of what a call costs. A mine inside the cache window reads
                // it at a fraction of the price — and mining clusters, so the
                // denser the session the more of them hit.
                cache_system: true,
            },
        )
        .await
        .map_err(Into::into)
}

/// Post-clean a raw gloss into the card-back HTML. The model returns a short
/// English meaning/usage block and a register keyword on a final line; this
/// trims wrapping quotes/whitespace, drops blank lines, and joins the remaining
/// lines with `<br>` (plain newlines don't render in Anki's HTML). See
/// the field format.
///
/// Opus-tier models (Opus 5 especially) sometimes echo the prompt's `<meaning>`/
/// `<usage>` schema placeholders as literal tags; strip them defensively so they
/// can never reach the card even if the prompt rule is ignored.
fn clean_gloss(raw: &str) -> String {
    let raw = raw
        .replace("<meaning>", "")
        .replace("</meaning>", "")
        .replace("<usage>", "")
        .replace("</usage>", "");
    raw.split('\n')
        .map(|line| {
            line.trim()
                .trim_matches(|c: char| matches!(c, '「' | '」' | '『' | '』' | '"' | '\''))
                .trim()
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("<br>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_gloss_joins_tag_line_with_br() {
        assert_eq!(
            clean_gloss("To wane — a gradual weakening.\nCOMMON · FORMAL"),
            "To wane — a gradual weakening.<br>COMMON · FORMAL"
        );
    }

    #[test]
    fn clean_gloss_strips_wrapping_quotes_and_blank_lines() {
        assert_eq!(
            clean_gloss("\"incinerator\"\n\nCOMMON · PLAIN"),
            "incinerator<br>COMMON · PLAIN"
        );
    }

    #[test]
    fn clean_gloss_empty_stays_empty() {
        assert_eq!(clean_gloss("   "), "");
    }

    #[test]
    fn parse_batch_reads_the_models_numbering() {
        let out = "1. to wane ## COMMON · PLAIN\n\n3. a forgery ## FORMAL · TECHNICAL\n";
        assert_eq!(
            parse_batch(out, 3),
            vec![
                Some("to wane<br>COMMON · PLAIN".into()),
                None,
                Some("a forgery<br>FORMAL · TECHNICAL".into()),
            ]
        );
    }

    /// A number outside the batch is dropped rather than panicking on the index.
    #[test]
    fn parse_batch_ignores_junk_lines() {
        let out = "Here are the glosses:\n1. camel ## CORE · PLAIN\n9. stray ## PLAIN\n";
        assert_eq!(
            parse_batch(out, 2),
            vec![Some("camel<br>CORE · PLAIN".into()), None]
        );
    }

    #[test]
    fn clean_gloss_strips_literal_meaning_tags() {
        // Newline-separated (Opus 4.8 shape) with echoed placeholder tags.
        assert_eq!(
            clean_gloss("<meaning>a forgery</meaning>\nCOMMON · FORMAL"),
            "a forgery<br>COMMON · FORMAL"
        );
        // Opus 5's single-line shape: tags + its own <br> before the tag line.
        assert_eq!(
            clean_gloss("<meaning>camel</meaning><br>CORE · PLAIN"),
            "camel<br>CORE · PLAIN"
        );
    }

    /// The case the surface-only shape was built for: a word whose kanji is rare
    /// and whose kana phrase is not. Prints both so the anchoring can be seen;
    /// asserts only that the written form is not tagged *below* the headword,
    /// since a tag tier is a model judgement and not a fixture.
    #[tokio::test]
    #[ignore = "requires KOTODEX_ANTHROPIC_API_KEY"]
    async fn the_written_form_is_not_priced_as_its_kanji() {
        let provider = Provider::from_env().expect("set KOTODEX_ANTHROPIC_API_KEY");
        let http = reqwest::Client::new();
        let sentence =
            |t: &str| format!("湿度が高く、薄暗く、ベッドなどの家具は硬く、<b>{t}</b>臭いがする。");
        let tier = |gloss: &str| {
            let tags = gloss.rsplit_once("<br>").expect("tag line").1.to_string();
            let fam = tags.split('·').next().unwrap().trim().to_string();
            ["OBSCURE", "RARE", "UNCOMMON", "COMMON", "CORE"]
                .iter()
                .position(|t| *t == fam)
                .unwrap_or_else(|| panic!("unknown familiarity: {gloss}"))
        };

        let surface = compact_def(&http, &provider, "すえた", &sentence("すえた"))
            .await
            .unwrap();
        let kanji = compact_def(&http, &provider, "饐えた", &sentence("饐えた"))
            .await
            .unwrap();
        println!("すえた: {surface}\n饐えた: {kanji}");
        assert!(
            tier(&surface) >= tier(&kanji),
            "the kana spelling should not be rarer than the kanji one"
        );
    }

    #[tokio::test]
    #[ignore = "requires KOTODEX_ANTHROPIC_API_KEY"]
    async fn compact_def_integration() {
        let provider = Provider::from_env().expect("set KOTODEX_ANTHROPIC_API_KEY");
        let http = reqwest::Client::new();
        let out = compact_def(
            &http,
            &provider,
            "減退する",
            "見た目も味も最悪な料理に食欲は<b>減退する</b>が、エマも口に運ぶ。",
        )
        .await
        .unwrap();
        assert!(!out.is_empty());
        assert!(out.chars().count() < 300, "should be compact, got: {out}");
        // The <meaning>/<usage> schema placeholders must never leak onto the card.
        assert!(
            !out.contains("<meaning>") && !out.contains("<usage>"),
            "tag leak: {out}"
        );
        // Meaning line, then a two-axis tag line, joined by <br>.
        let (_, tags) = out.rsplit_once("<br>").expect("expected a <br> tag line");
        let fam = tags
            .split('·')
            .next()
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();
        assert!(
            ["CORE", "COMMON", "UNCOMMON", "RARE", "OBSCURE"].contains(&fam),
            "tag line should start with a familiarity token, got: {out}"
        );
        println!("CompactDef: {out}");
    }
}
