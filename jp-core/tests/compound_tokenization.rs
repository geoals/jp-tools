use std::collections::HashSet;
use std::path::Path;

use jp_core::tokenize::{SudachiTokenizer, Tokenizer};

fn parse_headwords(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let word = line.split_whitespace().next()?;
            Some(word.to_string())
        })
        .collect()
}

#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn mode_c_with_headwords_keeps_compounds_that_mode_b_splits() {
    let dict_path = std::env::var("JP_TOOLS_SUDACHI_DICT_PATH")
        .expect("JP_TOOLS_SUDACHI_DICT_PATH must be set");
    let path = Path::new(&dict_path);

    let words = parse_headwords(include_str!("compound_headwords.txt"));
    let headword_set: HashSet<String> = words.iter().cloned().collect();
    assert_eq!(words.len(), 150, "expected 150 headwords in test data");

    let mode_b = SudachiTokenizer::new(path, HashSet::new()).unwrap();
    let mode_c = SudachiTokenizer::new(path, headword_set).unwrap();

    let mut kept_by_both = Vec::new();
    let mut split_by_both = Vec::new();
    let mut improved = Vec::new(); // split in B, kept in C

    for word in &words {
        let b_tokens = mode_b.tokenize(word).unwrap();
        let c_tokens = mode_c.tokenize(word).unwrap();
        let b_single = b_tokens.len() == 1;
        let c_single = c_tokens.len() == 1;

        match (b_single, c_single) {
            (true, true) => kept_by_both.push(word.as_str()),
            (false, true) => {
                let b_surfaces: Vec<_> = b_tokens.iter().map(|t| t.surface.as_str()).collect();
                improved.push((word.as_str(), b_surfaces.join(" + ")));
            }
            _ => {
                let surfaces: Vec<_> = c_tokens.iter().map(|t| t.surface.as_str()).collect();
                split_by_both.push((word.as_str(), surfaces.join(" + ")));
            }
        }
    }

    eprintln!("\n=== Mode C + headwords vs Mode B ===");
    eprintln!("Already single in Mode B:  {}", kept_by_both.len());
    eprintln!("Improved (B splits, C keeps): {}", improved.len());
    eprintln!("Still split in both:       {}", split_by_both.len());

    if !improved.is_empty() {
        eprintln!("\nImproved compounds (Mode B split → Mode C kept):");
        for (word, b_split) in &improved {
            eprintln!("  {word}  (was: {b_split})");
        }
    }

    if !split_by_both.is_empty() {
        eprintln!("\nStill split in Mode C ({}):", split_by_both.len());
        for (word, surfaces) in &split_by_both {
            eprintln!("  {word} → {surfaces}");
        }
    }

    let c_kept = kept_by_both.len() + improved.len();
    assert!(
        c_kept > words.len() * 80 / 100,
        "Expected >80% compounds kept as single tokens with Mode C, got {c_kept}/{}",
        words.len()
    );
    assert!(
        improved.len() > 0,
        "Expected at least some compounds to be improved vs Mode B"
    );
}
