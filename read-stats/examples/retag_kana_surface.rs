//! One-off: re-tag mined cards whose CompactDef was written from the headword.
//!
//! Until 3d28f65 the gloss was generated from Yomitan's vocab field, so a card
//! read as すえた was tagged as 饐える — the kanji priced, not the word. This
//! walks the deck, finds the cards where that could have bitten (a kanji
//! headword whose sentence spelt it in kana), regenerates the gloss through the
//! production path, and prints the diff.
//!
//! Dry by default. `--write` is what actually touches Anki.
//!
//!     cargo run -p read-stats --example retag_kana_surface
//!     cargo run -p read-stats --example retag_kana_surface -- --write
//!
//! Kept in the tree because it is the repair for a defect the notes still
//! carry, and because a second orthography bug would want the same shape.

use jp_mine_core::compactdef;
use read_stats::services::anki;
use serde_json::{Value, json};

const ANKI_URL: &str = "http://localhost:8765";
const DECK: &str = "deck:Japanese";
const VOCAB_FIELD: &str = "VocabKanji";
const SENTENCE_FIELD: &str = "SentKanji";
const COMPACT_FIELD: &str = "CompactDef";

fn has_kanji(s: &str) -> bool {
    s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

async fn call(http: &reqwest::Client, action: &str, params: Value) -> Value {
    let body = json!({ "action": action, "version": 6, "params": params });
    let resp: Value = http
        .post(ANKI_URL)
        .json(&body)
        .send()
        .await
        .expect("AnkiConnect unreachable")
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
    let write = std::env::args().any(|a| a == "--write");
    let api_key =
        std::env::var("KOTODEX_ANTHROPIC_API_KEY").expect("set KOTODEX_ANTHROPIC_API_KEY");
    let http = reqwest::Client::new();

    let ids = call(&http, "findNotes", json!({ "query": DECK })).await;
    let notes = call(&http, "notesInfo", json!({ "notes": ids })).await;
    let notes = notes.as_array().expect("notesInfo returned no array");

    // The exposed class: the headword carries kanji the sentence did not use.
    // Where both are spelt the same there was nothing for the old path to get
    // wrong, so those cards are left alone rather than re-billed.
    let mut targets = Vec::new();
    for note in notes {
        let field = |name: &str| {
            note["fields"][name]["value"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        };
        let (raw_sentence, old) = (
            field(SENTENCE_FIELD),
            anki::clean_field(&field(COMPACT_FIELD)),
        );
        let vocab = anki::clean_field(&field(VOCAB_FIELD));
        let Some(surface) = anki::bolded_span(&raw_sentence) else {
            continue;
        };
        if old.is_empty() || !has_kanji(&vocab) || has_kanji(&surface) {
            continue;
        }
        let note_id = note["noteId"].as_i64().expect("note without an id");
        let sentence = anki::clean_field_keep_bold(&raw_sentence);
        targets.push((note_id, vocab, surface, sentence, field(COMPACT_FIELD)));
    }

    println!(
        "{} card(s) to re-tag{}\n",
        targets.len(),
        if write { "" } else { " (dry run)" }
    );

    let (mut changed, mut failed) = (0, 0);
    for (note_id, vocab, surface, sentence, old) in &targets {
        let new = match compactdef::compact_def(&http, &api_key, surface, sentence).await {
            Ok(def) if !def.is_empty() => def,
            Ok(_) => {
                println!("{vocab} → {surface}: EMPTY, skipped");
                failed += 1;
                continue;
            }
            Err(e) => {
                println!("{vocab} → {surface}: FAILED ({e})");
                failed += 1;
                continue;
            }
        };
        let tags = |s: &str| s.rsplit_once("<br>").map(|(_, t)| t.to_string());
        let moved = tags(old) != tags(&new);
        if moved {
            changed += 1;
        }
        println!(
            "{} {vocab} → {surface}\n    was: {old}\n    now: {new}",
            if moved { "~" } else { " " }
        );
        if write {
            match anki::update_note_field_verified(&http, ANKI_URL, *note_id, COMPACT_FIELD, &new)
                .await
            {
                Ok(()) => {}
                Err(e) => {
                    println!("    WRITE FAILED: {e}");
                    failed += 1;
                }
            }
        }
    }

    println!(
        "\n{}/{} tag line(s) moved, {failed} failure(s){}",
        changed,
        targets.len(),
        if write {
            ", written"
        } else {
            ", nothing written"
        }
    );
}
