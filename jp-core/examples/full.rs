//! The production tokenizer, run over lines given on stdin.
//!
//! ```text
//! cargo run --example full -p jp-core -- <master.tsv> <conj.tsv> <anki.tsv> <dict.dic> < lines.txt
//! ```

use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

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

    let tk = SudachiTokenizer::new(Path::new(&a[4]), anki)
        .unwrap()
        .with_lexicon(lexicon)
        .with_master_readings(&entries)
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
