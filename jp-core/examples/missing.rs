//! Ad-hoc probe: which master-dictionary headwords does Sudachi's own lexicon
//! not hold as a single Mode C morpheme?
//!
//! ```text
//! cargo run --release --example missing -p jp-core -- <terms.txt> <dict.dic>
//! ```

use std::path::Path;

use sudachi::analysis::Tokenize;
use sudachi::analysis::stateless_tokenizer::StatelessTokenizer;
use sudachi::config::Config;
use sudachi::dic::dictionary::JapaneseDictionary;
use sudachi::prelude::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let terms = std::fs::read_to_string(&args[1]).unwrap();
    let config = Config::new(None, None, Some(Path::new(&args[2]).to_path_buf())).unwrap();
    let dict = JapaneseDictionary::from_cfg(&config).unwrap();
    let tokenizer = StatelessTokenizer::new(&dict);

    let (mut whole, mut split) = (0usize, 0usize);
    for term in terms.lines().map(str::trim).filter(|t| !t.is_empty()) {
        let Ok(ms) = tokenizer.tokenize(term, Mode::C, false) else {
            continue;
        };
        if ms.len() == 1 {
            whole += 1;
        } else {
            split += 1;
            println!(
                "{term}\t{}",
                ms.iter()
                    .map(|m| m.surface().to_string())
                    .collect::<Vec<_>>()
                    .join("+")
            );
        }
    }
    eprintln!(
        "{whole} whole, {split} split ({:.1}% of {})",
        100.0 * split as f64 / (whole + split) as f64,
        whole + split
    );
}
