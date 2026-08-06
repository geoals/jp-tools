//! Every line's token stream, for diffing one tokenizer rule against another.
//!
//! The golden fixture covers 250 sampled lines and is there to be *read*; this
//! is the whole corpus, so a rule change can be checked for the lines it moved
//! that it was not meant to move. Run it before and after, diff the two.
//!
//! ```text
//! cargo run --release --example tokens -p jp-core -- <knowledge.db> <sudachi.dic> [dictionary title...]
//! ```
//!
//! Extra dictionary titles widen the lexicon the tokenizer segments by, which
//! is the master alone in production. That is the "should 明鏡 decide wordhood
//! too" question, asked as a diff.
//!
//! One line per line read: the text, then the identities, tab separated.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use jp_core::knowledge::Knowledge;
use jp_core::knowledge::dictionaries;
use jp_core::tokenize::{SudachiTokenizer, Tokenizer};

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(run());
}

async fn run() {
    let a: Vec<String> = std::env::args().collect();
    let k = Knowledge::open(&a[1]).await.unwrap();
    let pool = k.pool();

    // The same five inputs the reader and the ingest build their tokenizer
    // with — a tokenizer missing any of them is a second pipeline and its
    // output is not what production produces.
    let mut entries = dictionaries::master_entries(pool).await.unwrap();
    let mut conjugatable = dictionaries::master_conjugatable(pool).await.unwrap();
    for title in &a[3..] {
        let d = dictionaries::by_title(pool, title)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("no dictionary titled {title}"));
        let extra: Vec<(String, String)> = sqlx::query_as(
            "SELECT DISTINCT term, reading FROM dictionary_entries \
             WHERE dictionary_id = ? AND reading != ''",
        )
        .bind(d.id)
        .fetch_all(pool)
        .await
        .unwrap();
        eprintln!("+ {title}: {} entries", extra.len());
        entries.extend(extra);
        let conj: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT term FROM dictionary_entries WHERE dictionary_id = ? AND rules != ''",
        )
        .bind(d.id)
        .fetch_all(pool)
        .await
        .unwrap();
        conjugatable.extend(conj.into_iter().map(|(t,)| t));
    }
    entries.sort();
    entries.dedup();
    let lexicon: HashSet<String> = entries.iter().map(|(t, _)| t.clone()).collect();
    let ranks = match dictionaries::by_title(pool, "BCCWJ").await.unwrap() {
        Some(d) => {
            let terms = jp_core::tokenize::ambiguous_headwords(&entries);
            dictionaries::frequency_ranks(pool, d.id, &terms)
                .await
                .unwrap()
        }
        None => HashMap::new(),
    };
    let preferred = match (
        dictionaries::master(pool).await.unwrap(),
        dictionaries::by_title(pool, "Jitendex").await.unwrap(),
        dictionaries::by_title(pool, "BCCWJ").await.unwrap(),
    ) {
        (Some(m), Some(j), Some(b)) => dictionaries::preferred_readings(pool, m.id, j.id, b.id)
            .await
            .unwrap(),
        _ => HashMap::new(),
    };
    let mined: HashSet<String> = sqlx::query_scalar("SELECT DISTINCT vocab FROM anki_notes")
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .collect();

    let tk = SudachiTokenizer::new(Path::new(&a[2]), mined)
        .unwrap()
        .with_lexicon(lexicon)
        .with_master_readings(&entries)
        .with_frequency(ranks)
        .with_preferred_readings(preferred)
        .with_conjugatable(conjugatable);

    let lines: Vec<String> =
        sqlx::query_scalar("SELECT text FROM lines WHERE discarded = 0 ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap();
    for text in &lines {
        let Ok(tokens) = tk.tokenize(text) else {
            continue;
        };
        let out: Vec<String> = tokens
            .iter()
            .map(|t| format!("{}/{}", t.surface, t.base_form))
            .collect();
        println!("{text}\t{}", out.join(" "));
    }
}
