//! Audit — and repair — the CompactDef tag line on every card in the collection,
//! through the same [`TagLine`] parser the mining path now enforces.
//!
//! Three modes, in the order they should be run:
//!
//!     cargo run -p read-stats --example canonical_tags            # report only
//!     cargo run -p read-stats --example canonical_tags -- --fix   # separator slips
//!     cargo run -p read-stats --example canonical_tags -- --retag # re-judge the rest
//!
//! `--fix` is free and cannot change a judgement: it only re-renders what parsed.
//! `--retag` bills one Opus call per card, and is for the lines the parser
//! rejects — a missing baseline, two baselines, an invented tag. Those are
//! judgements that were never validly made, so they are made again through the
//! production path rather than patched here.
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
    let arg = |name: &str| std::env::args().any(|a| a == name);
    let (fix, retag) = (arg("--fix"), arg("--retag"));

    // AnkiConnect closes the connection without saying so, so a pooled one fails
    // on the next write in a long run of them.
    let http = reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .build()
        .expect("failed to build the HTTP client");

    let ids = call(&http, "findNotes", json!({ "query": "CompactDef:_*" })).await;
    let notes = call(&http, "notesInfo", json!({ "notes": ids })).await;
    let notes = notes.as_array().expect("notesInfo returned no array");

    let (mut ok, mut repaired, mut rejected, mut failed) = (0, 0, 0, 0);
    for note in notes {
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
                continue;
            }
            Ok(parsed) => {
                repaired += 1;
                println!("REPAIR {vocab} — {tags}  →  {parsed}");
                if fix {
                    let new = format!("{meaning}<br>{parsed}");
                    if let Err(e) =
                        anki::update_note_field_verified(&http, ANKI_URL, note_id, COMPACT_FIELD, &new)
                            .await
                    {
                        println!("    WRITE FAILED: {e}");
                        failed += 1;
                    }
                }
                continue;
            }
            Err(why) => why,
        };

        rejected += 1;
        println!("REJECT {vocab} — {why}: {tags}");
        if !retag {
            continue;
        }

        // Re-judged from the same inputs the mining path uses: the surface as
        // the sentence spelt it, never the headword.
        let raw_sentence = field("SentKanji");
        let surface = anki::bolded_span(raw_sentence).unwrap_or_else(|| vocab.clone());
        let sentence = anki::clean_field_keep_bold(raw_sentence);
        let api_key =
            std::env::var("JP_TOOLS_ANTHROPIC_API_KEY").expect("set JP_TOOLS_ANTHROPIC_API_KEY");

        match compactdef::compact_def(&http, &api_key, &surface, &sentence).await {
            Ok(new) if !new.is_empty() => {
                println!("    retagged: {new}");
                if let Err(e) =
                    anki::update_note_field_verified(&http, ANKI_URL, note_id, COMPACT_FIELD, &new)
                        .await
                {
                    println!("    WRITE FAILED: {e}");
                    failed += 1;
                }
            }
            Ok(_) => {
                println!("    RETAG EMPTY, left alone");
                failed += 1;
            }
            Err(e) => {
                println!("    RETAG FAILED: {e}");
                failed += 1;
            }
        }
    }

    let repaired_verb = if fix { "repaired" } else { "repairable" };
    let rejected_verb = if retag { "re-tagged" } else { "rejected" };
    println!(
        "\n{} cards: {ok} canonical, {repaired} {repaired_verb}, {rejected} {rejected_verb}, \
         {failed} failure(s)",
        notes.len()
    );
}
