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

/// Sudachi has no entry for these, so they arrive as a content word plus a
/// trailing 接尾辞 — 度し(動詞) + 難い(接尾辞). The parts spell the master
/// headword exactly, which is the strongest signal recomposition has, and it
/// was being refused because the suffix is not a content word.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_trailing_suffix_may_spell_a_listed_headword() {
    let dict_path = std::env::var("KOTODEX_SUDACHI_DICT_PATH")
        .expect("KOTODEX_SUDACHI_DICT_PATH must be set");
    // Each is spelt by its parts as written, the last one in its base form —
    // 怖がり would need 怖がる listed instead, which is a different rung.
    let listed = ["度し難い", "言い難い", "得難い", "行き方"];
    let tokenizer = SudachiTokenizer::new(Path::new(&dict_path), HashSet::new())
        .unwrap()
        .with_lexicon(listed.iter().map(|w| w.to_string()).collect());

    for word in listed {
        let tokens = tokenizer.tokenize(word).unwrap();
        let surfaces: Vec<_> = tokens.iter().map(|t| t.surface.as_str()).collect();
        assert_eq!(surfaces, [word], "{word} should survive as one token");
    }

    // The lexicon is what admits the join, not the suffix tag: an unlisted
    // compound of the same shape still comes apart.
    let bare = SudachiTokenizer::new(Path::new(&dict_path), HashSet::new())
        .unwrap()
        .with_lexicon(HashSet::from(["難い".to_string()]));
    assert!(bare.tokenize("度し難い").unwrap().len() > 1);
}

/// An idiom with particles inside is four or five morphemes, and at a cap of
/// three recomposition never offered the run at all — the master listed the
/// word and nothing ever asked about it.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn an_idiom_longer_than_three_morphemes_is_still_offered() {
    let dict_path = std::env::var("KOTODEX_SUDACHI_DICT_PATH")
        .expect("KOTODEX_SUDACHI_DICT_PATH must be set");
    let listed = ["一か八か", "首を横に振る", "何でもない"];
    let tokenizer = SudachiTokenizer::new(Path::new(&dict_path), HashSet::new())
        .unwrap()
        .with_lexicon(listed.iter().map(|w| w.to_string()).collect());

    for word in listed {
        let tokens = tokenizer.tokenize(word).unwrap();
        let surfaces: Vec<_> = tokens.iter().map(|t| t.surface.as_str()).collect();
        assert_eq!(surfaces, [word], "{word} should survive as one token");
    }

    let sentence = tokenizer.tokenize("彼は一か八かの勝負に出た").unwrap();
    assert!(
        sentence.iter().any(|t| t.surface == "一か八か"),
        "the idiom should survive inside a sentence too"
    );
}

#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn mode_c_with_headwords_keeps_compounds_that_mode_b_splits() {
    let dict_path = std::env::var("KOTODEX_SUDACHI_DICT_PATH")
        .expect("KOTODEX_SUDACHI_DICT_PATH must be set");
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
        !improved.is_empty(),
        "Expected at least some compounds to be improved vs Mode B"
    );
}
