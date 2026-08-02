//! The production tokenizer, run over lines given on stdin.
//!
//! ```text
//! cargo run --example full -p jp-core -- <master.tsv> <conj.tsv> <anki.tsv> <dict.dic> <freq.tsv> <prefer.tsv> < lines.txt
//!
//! All six, for the reason `surfaces.rs` says: without the ranks and the
//! preferences the identity ladder's last steps cannot fire and the output is
//! not what production produces.
//! ```

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;

use jp_core::knowledge::dictionaries::PreferredReading;
use jp_core::tokenize::{SudachiTokenizer, Tokenizer};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let master = std::fs::read_to_string(&a[1]).unwrap();
    let entries: Vec<(String, String)> = master
        .lines()
        .filter_map(|l| {
            let (t, r) = l.split_once('\t')?;
            Some((t.to_string(), r.to_string()))
        })
        .collect();
    let lexicon: HashSet<String> = entries.iter().map(|(t, _)| t.clone()).collect();
    let conj: HashSet<String> = std::fs::read_to_string(&a[2])
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    let anki: HashSet<String> = std::fs::read_to_string(&a[3])
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();

    let ranks: HashMap<(String, String), i64> = std::fs::read_to_string(&a[5])
        .unwrap()
        .lines()
        .filter_map(|l| {
            let mut it = l.split('\t');
            let term = it.next()?.to_string();
            let reading = jp_core::text::kana::to_hiragana(it.next()?);
            Some(((term, reading), it.next()?.trim().parse().ok()?))
        })
        .collect();
    let prefer: HashMap<String, PreferredReading> = std::fs::read_to_string(&a[6])
        .unwrap()
        .lines()
        .filter_map(|l| {
            let mut it = l.split('\t');
            let term = it.next()?.to_string();
            let preferred = it.next()?.to_string();
            let acceptable = it.next()?.split(',').map(str::to_string).collect();
            Some((
                term,
                PreferredReading {
                    preferred,
                    acceptable,
                },
            ))
        })
        .collect();
    let tk = SudachiTokenizer::new(Path::new(&a[4]), anki)
        .unwrap()
        .with_lexicon(lexicon)
        .with_master_readings(&entries)
        .with_frequency(ranks)
        .with_preferred_readings(prefer)
        .with_conjugatable(conj);

    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    for line in input.lines() {
        println!("== {line}");
        for t in tk.tokenize(line).unwrap() {
            println!(
                "   {:<8} -> {:<10} {:<10} {}",
                t.surface, t.base_form, t.reading, t.pos
            );
        }
    }
}
