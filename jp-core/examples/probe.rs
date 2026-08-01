//! Ad-hoc probe: tokenize with the production setup and show what identity each
//! token gets.
//!
//! ```text
//! cargo run --example probe -p jp-core -- <master.tsv> <anki.tsv> <dict.dic> [--corpus lines.txt]
//! ```
//!
//! `--corpus` prints only the tokens whose identity the master does *not* list,
//! one per line and sorted by frequency, which is the before/after diff for a
//! change to identity resolution. Build the input with
//! `sqlite3 knowledge.db "select text from lines" > lines.txt`, and the ranks
//! with `select term, min(frequency) ... where dictionary_id = <bccwj>`.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use jp_core::knowledge::dictionaries::PreferredReading;
use jp_core::tokenize::{
    MasterWords, SudachiTokenizer, Tokenizer, ambiguous_headwords, counts_as_word,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let master_tsv = std::fs::read_to_string(&args[1]).unwrap();
    let anki_tsv = std::fs::read_to_string(&args[2]).unwrap();
    let dict_path = Path::new(&args[3]);
    let corpus = args
        .iter()
        .position(|a| a == "--corpus")
        .map(|i| &args[i + 1]);
    let freq_tsv = args
        .iter()
        .position(|a| a == "--freq")
        .map(|i| std::fs::read_to_string(&args[i + 1]).unwrap());

    let entries: Vec<(String, String)> = master_tsv
        .lines()
        .filter_map(|l| {
            let mut it = l.split('\t');
            Some((it.next()?.to_string(), it.next().unwrap_or("").to_string()))
        })
        .collect();
    let lexicon: HashSet<String> = entries.iter().map(|(t, _)| t.clone()).collect();
    let anki: HashSet<String> = anki_tsv.lines().map(|l| l.trim().to_string()).collect();
    let ambiguous: HashSet<String> = ambiguous_headwords(&entries).into_iter().collect();
    let ranks: HashMap<String, i64> = freq_tsv
        .iter()
        .flat_map(|s| s.lines())
        .filter_map(|l| {
            let (term, rank) = l.split_once('\t')?;
            if !ambiguous.contains(term) {
                return None;
            }
            Some((term.to_string(), rank.trim().parse().ok()?))
        })
        .collect();

    // As dumped by the `preferences` example: term, preferred, acceptable list.
    let prefer_tsv = args
        .iter()
        .position(|a| a == "--prefer")
        .map(|i| std::fs::read_to_string(&args[i + 1]).unwrap());
    let prefer: HashMap<String, PreferredReading> = prefer_tsv
        .iter()
        .flat_map(|s| s.lines())
        .filter_map(|l| {
            let mut it = l.split('\t');
            let term = it.next()?.to_string();
            let preferred = it.next()?.to_string();
            let acceptable = it.next()?.split(',').map(|s| s.to_string()).collect();
            Some((
                term,
                PreferredReading {
                    preferred,
                    acceptable,
                },
            ))
        })
        .collect();

    let tokenizer = SudachiTokenizer::new(dict_path, anki)
        .unwrap()
        .with_lexicon(lexicon.clone())
        .with_master_readings(&entries)
        .with_frequency(ranks)
        .with_preferred_readings(prefer);
    let master = MasterWords::new(lexicon, &entries);

    let Some(corpus) = corpus else {
        for line in [
            "そのメルルの胸を、とんっと軽く押した。",
            "それどころか、彼は笑っていた。",
            "敵の隙をうかがう",
            "食べてみる",
            "行かなければならない",
            "私なら行けるはずだ",
            "ちょっと待って",
            "綺麗ごとを言うな",
        ] {
            println!("== {line}");
            for t in tokenizer.tokenize(line).unwrap() {
                println!(
                    "  {:<10} base={:<10} read={:<12} pos={:<6} word={}",
                    t.surface,
                    t.base_form,
                    t.reading,
                    t.pos,
                    counts_as_word(&t, &master)
                );
            }
        }
        return;
    };

    let text = std::fs::read_to_string(corpus).unwrap();
    let mut unlisted: HashMap<(String, String), usize> = HashMap::new();
    let (mut counted, mut total) = (0usize, 0usize);
    for line in text.lines() {
        for t in tokenizer.tokenize(line).unwrap() {
            if !counts_as_word(&t, &master) {
                continue;
            }
            counted += 1;
            total += 1;
            if std::env::var("PROBE_ALL").is_ok() || !master.lists(&t.base_form, &t.reading) {
                *unlisted
                    .entry((t.base_form.clone(), t.reading.clone()))
                    .or_default() += 1;
            }
        }
    }
    let mut rows: Vec<_> = unlisted.into_iter().collect();
    rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    let off_scale: usize = rows.iter().map(|(_, n)| n).sum();
    eprintln!(
        "{counted} counted tokens, {off_scale} off the master scale ({:.1}%), {} distinct",
        100.0 * off_scale as f64 / total.max(1) as f64,
        rows.len()
    );
    for ((term, reading), n) in rows {
        println!("{n}\t{term}\t{reading}");
    }
}
