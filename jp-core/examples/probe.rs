//! Ad-hoc probe: tokenize sample lines with the production setup.
//! cargo run --example probe -p jp-core -- <master.tsv> <anki.tsv> <dict.dic>

use std::collections::HashSet;
use std::path::Path;

use jp_core::tokenize::{counts_as_word, MasterWords, SudachiTokenizer, Tokenizer};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let master_tsv = std::fs::read_to_string(&args[1]).unwrap();
    let anki_tsv = std::fs::read_to_string(&args[2]).unwrap();
    let dict_path = Path::new(&args[3]);

    let entries: Vec<(String, String)> = master_tsv
        .lines()
        .filter_map(|l| {
            let mut it = l.split('\t');
            Some((it.next()?.to_string(), it.next().unwrap_or("").to_string()))
        })
        .collect();
    let lexicon: HashSet<String> = entries.iter().map(|(t, _)| t.clone()).collect();
    let anki: HashSet<String> = anki_tsv.lines().map(|l| l.trim().to_string()).collect();

    let tokenizer = SudachiTokenizer::new(dict_path, anki)
        .unwrap()
        .with_lexicon(lexicon.clone())
        .with_master_readings(&entries);
    let master = MasterWords::new(lexicon, &entries);

    let lines = [
        "そのメルルの胸を、とんっと軽く押した。",
        "それどころか、彼は笑っていた。",
        "敵の隙をうかがう",
        "食べてみる",
        "行かなければならない",
        "私なら行けるはずだ",
        "ちょっと待って",
        "綺麗ごとを言うな",
    ];

    for line in lines {
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
}
