//! Audit — and repair — the CompactDef tag line on every card in the collection,
//! through the same [`TagLine`] parser the mining path now enforces.
//!
//! Modes, in the order they should be run:
//!
//!     ... --example canonical_tags                  # report only
//!     ... --example canonical_tags -- --fix         # separator slips, free
//!     ... --example canonical_tags -- --retag       # re-judge what fails to parse
//!     ... --example canonical_tags -- --retag-all   # re-judge every card
//!
//! `--fix` cannot change a judgement: it only re-renders what already parsed.
//! `--retag` re-judges the lines the parser rejects — a missing baseline, two
//! baselines, an invented tag — because those are judgements that were never
//! validly made. `--retag-all` also re-judges the cards that parse but were
//! written under an older rubric, which is the only way to get one judge over
//! the whole collection.
//!
//! Re-judging goes through the `claude` CLI by default so the work is billed to
//! the subscription, a batch of `--batch N` cards (default 20) per call; `--api`
//! uses API credits and one call per card instead. The sweep is resumable —
//! re-tagged cards parse and are skipped on the next run — so a run stopped by a
//! usage limit is picked up simply by running it again.
//!
//! `--retag-all` is the exception: a re-judged card parses, so nothing tells the
//! next run it is done. The pass walks the collection in note-id order and
//! `--skip N` resumes it — after `--limit 550`, continue with `--skip 550`.
//!
//! Kept in the tree because a prompt change can reintroduce the class, and the
//! report is how you find out.

use jp_mine_core::compactdef;
use jp_mine_core::tags::TagLine;
use read_stats::services::anki;
use serde_json::{Value, json};

const ANKI_URL: &str = "http://localhost:8765";
const COMPACT_FIELD: &str = "CompactDef";

async fn call(http: &reqwest::Client, action: &str, params: Value) -> Value {
    let body = json!({ "action": action, "version": 6, "params": params });
    let resp: Value = http
        .post(ANKI_URL)
        .json(&body)
        .send()
        .await
        .expect("AnkiConnect unreachable — is Anki running?")
        .json()
        .await
        .expect("AnkiConnect response unparseable");
    assert!(
        resp["error"].is_null(),
        "AnkiConnect {action} failed: {}",
        resp["error"]
    );
    resp["result"].clone()
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let arg = |name: &str| std::env::args().any(|a| a == name);
    let (fix, api) = (arg("--fix"), arg("--api"));
    let retag_all = arg("--retag-all");
    let retag = retag_all || arg("--retag");
    // Re-judging is the only expensive step, so it is the only one worth
    // stopping early: --limit N re-tags the first N and reports the rest.
    let limit = std::env::args()
        .skip_while(|a| a != "--limit")
        .nth(1)
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(usize::MAX);
    // --retag-all re-judges cards that already parse, so nothing marks them
    // done: --skip N is the resume point, and the pass is note-id ordered to
    // make it mean the same thing on the next run.
    let skip = std::env::args()
        .skip_while(|a| a != "--skip")
        .nth(1)
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0);
    // How many cards share one CLI call. The system prompt is ~1,300 tokens and
    // is resent per call, so a batch of 20 pays it once for 20 cards.
    let batch_size = std::env::args()
        .skip_while(|a| a != "--batch")
        .nth(1)
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(20);

    // AnkiConnect closes the connection without saying so, so a pooled one fails
    // on the next write in a long run of them.
    let http = reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .build()
        .expect("failed to build the HTTP client");

    let ids = call(&http, "findNotes", json!({ "query": "CompactDef:_*" })).await;
    let notes = call(&http, "notesInfo", json!({ "notes": ids })).await;
    let mut notes = notes
        .as_array()
        .expect("notesInfo returned no array")
        .clone();
    notes.sort_by_key(|n| n["noteId"].as_i64().unwrap_or_default());

    let (mut ok, mut repaired, mut rejected, mut failed) = (0, 0, 0, 0);
    // (note_id, vocab, target, sentence) for every card to be re-judged. Built
    // in full first so the expensive half is one predictable pass with a known
    // total, instead of an LLM call interleaved with the walk.
    let mut todo: Vec<(i64, String, String, String)> = Vec::new();
    let mut candidates = 0usize;
    for note in &notes {
        let field = |name: &str| note["fields"][name]["value"].as_str().unwrap_or_default();
        let note_id = note["noteId"].as_i64().expect("note without an id");
        let vocab = anki::clean_field(field("VocabKanji"));
        let value = field(COMPACT_FIELD);

        let Some((meaning, tags)) = value.rsplit_once("<br>") else {
            rejected += 1;
            println!("REJECT {vocab} — no tag line: {value}");
            continue;
        };

        let why = match TagLine::parse(tags) {
            Ok(parsed) if parsed.to_string() == tags.trim() => {
                ok += 1;
                if !retag_all {
                    continue;
                }
                String::from("re-judging under the current rubric")
            }
            Ok(parsed) if !retag_all => {
                repaired += 1;
                println!("REPAIR {vocab} — {tags}  →  {parsed}");
                if fix {
                    let new = format!("{meaning}<br>{parsed}");
                    if let Err(e) = anki::update_note_field_verified(
                        &http,
                        ANKI_URL,
                        note_id,
                        COMPACT_FIELD,
                        &new,
                    )
                    .await
                    {
                        println!("    WRITE FAILED: {e}");
                        failed += 1;
                    }
                }
                continue;
            }
            Ok(_) => String::from("re-judging under the current rubric"),
            Err(why) => why.to_string(),
        };

        rejected += 1;
        println!("REJECT {vocab} — {why}: {tags}");
        if !retag || todo.len() >= limit {
            continue;
        }
        candidates += 1;
        if candidates <= skip {
            continue;
        }

        // Re-judged from the same inputs the mining path uses: the surface as
        // the sentence spelt it, never the headword.
        let raw_sentence = field("SentKanji");
        let surface = anki::bolded_span(raw_sentence).unwrap_or_else(|| vocab.clone());
        let sentence = anki::clean_field_keep_bold(raw_sentence);
        todo.push((note_id, vocab, surface, sentence));
    }

    let api_key = api.then(|| {
        std::env::var("JP_TOOLS_ANTHROPIC_API_KEY").expect("set JP_TOOLS_ANTHROPIC_API_KEY")
    });
    let mut retagged = 0usize;
    let mut stopped = false;
    for (batch_no, chunk) in todo.chunks(batch_size).enumerate() {
        println!(
            "\n-- batch {} ({}-{} of {})",
            batch_no + 1,
            batch_no * batch_size + 1,
            batch_no * batch_size + chunk.len(),
            todo.len()
        );

        let mut results = Vec::with_capacity(chunk.len());
        match &api_key {
            Some(key) => {
                for (_, _, surface, sentence) in chunk {
                    results.push(compactdef::compact_def(&http, key, surface, sentence).await);
                }
            }
            None => {
                let items: Vec<_> = chunk
                    .iter()
                    .map(|(_, _, surface, sentence)| (surface.clone(), sentence.clone()))
                    .collect();
                results = compactdef::compact_def_batch_cli(&items);
            }
        }

        for ((note_id, vocab, _, _), result) in chunk.iter().zip(results) {
            match result {
                Ok(new) if !new.is_empty() => {
                    retagged += 1;
                    println!("  {vocab}: {new}");
                    if let Err(e) = anki::update_note_field_verified(
                        &http,
                        ANKI_URL,
                        *note_id,
                        COMPACT_FIELD,
                        &new,
                    )
                    .await
                    {
                        println!("    WRITE FAILED: {e}");
                        failed += 1;
                    }
                }
                Ok(_) => {
                    println!("  {vocab}: RETAG EMPTY, left alone");
                    failed += 1;
                }
                // A backend failure is not about this card. Spawning another few
                // hundred `claude` processes into a usage limit is what the
                // previous run did; the sweep is resumable, so stopping costs
                // nothing but a rerun.
                Err(e @ compactdef::CompactDefError::Unavailable(_)) => {
                    println!("\nSTOPPING — {e}");
                    stopped = true;
                    break;
                }
                Err(e) => {
                    println!("  {vocab}: RETAG FAILED: {e}");
                    failed += 1;
                }
            }
        }
        if stopped {
            break;
        }
    }

    let repaired_verb = if fix { "repaired" } else { "repairable" };
    let rejected_verb = if retag { "re-tagged" } else { "rejected" };
    println!(
        "\n{} cards: {ok} canonical, {repaired} {repaired_verb}, {rejected} {rejected_verb}, \
         {retagged}/{} re-judged, {failed} failure(s){}",
        notes.len(),
        todo.len(),
        if stopped { " — STOPPED EARLY" } else { "" }
    );
}
