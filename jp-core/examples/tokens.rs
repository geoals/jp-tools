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
//! The `standard` role's dictionaries are loaded the way production loads them,
//! by role. Extra titles named on the command line widen the segmentation
//! authority on top of that, which is how a dictionary that is not in the role
//! yet gets asked "what would this decide?" as a diff.
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
    let entries = dictionaries::master_entries(pool).await.unwrap();
    let conjugatable = dictionaries::master_conjugatable(pool).await.unwrap();
    // The segmentation authority, added as what it is: these decide wordhood
    // beside the master and nothing else.
    //
    // **By role first.** Taking it from the command line alone meant every dump
    // ran without 明鏡 and 小学館 — a pipeline two dictionaries short of the one
    // the reader is using, so a rule diffed against itself was right while the
    // absolute picture was not production's.
    let mut standard: Vec<(String, String)> = dictionaries::standard_entries(pool).await.unwrap();
    eprintln!("standard role: {} entries", standard.len());
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
        standard.extend(extra);
    }
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

    // The rank per spelling, which the short-kana guard is the only consumer
    // of. Left out of this dump for its first year, so what it printed was a
    // pipeline with one rule switched off — see `Pipeline`'s nine inputs.
    let reader_ranks: HashMap<String, i64> =
        match dictionaries::reader_frequency(pool)
            .await
            .unwrap()
        {
            Some(d) => sqlx::query_as::<_, (String, i64)>(
                "SELECT term, MIN(frequency) FROM dictionary_frequency \
                 WHERE dictionary_id = ? GROUP BY term",
            )
            .bind(d.id)
            .fetch_all(pool)
            .await
            .unwrap()
            .into_iter()
            .collect(),
            None => HashMap::new(),
        };
    // `TOKENS_NAMES=off` drops the cast, so two runs differ in the name pass
    // and nothing else.
    let names: HashSet<String> = if std::env::var("TOKENS_NAMES").as_deref() == Ok("off") {
        HashSet::new()
    } else {
        jp_core::knowledge::work_names::all(&k)
            .await
            .unwrap()
            .into_iter()
            .collect()
    };

    let tk = SudachiTokenizer::new(Path::new(&a[2]), mined)
        .unwrap()
        .with_lexicon(lexicon)
        .with_master_readings(&entries)
        .with_frequency(ranks)
        .with_reader_frequency(reader_ranks)
        .with_preferred_readings(preferred)
        .with_conjugatable(conjugatable)
        .with_standard(&standard)
        .with_names(names);

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
            .map(|t| {
                let name = if t.proper_noun { "!" } else { "" };
                let reading = jp_core::text::kana::to_hiragana(&t.reading);
                format!("{}/{}/{reading}{name}", t.surface, t.base_form)
            })
            .collect();
        // One record per output line, always. 1,040 of the 33,949 lines read
        // carry a newline, and printed raw they came out as several rows whose
        // text column held the last fragment and whose tokens held the whole
        // line — so anything sampling this file read the wrong sentence for 3%
        // of it. Escaped after joining, because the surface, the headword and
        // the reading can each be a newline too. The tokenizer still sees the
        // line whole.
        let row = format!("{text}\t{}", out.join(" "));
        println!("{}", row.replace(['\n', '\r'], "⏎"));
    }
}
