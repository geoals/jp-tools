//! The tokenizer's output over a fixed corpus, as text a human can review.
//!
//! Every rule in `tokenize.rs` trades one kind of mistake for another, and the
//! only way to see the trade is to look at what changes across real sentences.
//! This renders that, so the fixture under `tests/golden/` is a diff rather than
//! a claim: change a rule, run the test, read what moved.
//!
//! Shared by `tests/golden.rs`, which verifies, and `examples/golden.rs`, which
//! regenerates — the two must build the tokenizer the same way or the fixture
//! means nothing.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::knowledge::dictionaries::PreferredReading;
use crate::text::kana::to_hiragana;
use crate::tokenize::{MasterWords, SudachiTokenizer, Tokenizer, counts_as_word};

/// Everything the tokenizer is configured from, as one value.
///
/// A struct rather than positional arguments because four of them are a
/// `HashSet<String>` or a `HashMap<String, _>`: a swapped pair would still
/// compile and would quietly build a different pipeline, which is the one
/// failure this whole file exists to catch.
#[derive(Default)]
pub struct Inputs {
    pub master: Vec<(String, String)>,
    pub standard: Vec<(String, String)>,
    pub ranks: HashMap<(String, String), i64>,
    pub reader_ranks: HashMap<String, i64>,
    pub preferences: HashMap<String, PreferredReading>,
    pub conjugatable: HashSet<String>,
    pub names: HashSet<String>,
}

/// Build the tokenizer exactly as `kotodex-server` ingest does.
pub fn tokenizer(dict_path: &Path, i: &Inputs) -> SudachiTokenizer {
    let lexicon: HashSet<String> = i.master.iter().map(|(t, _)| t.clone()).collect();
    // A non-empty deck is what puts it on the C→B→A path that production runs.
    SudachiTokenizer::new(dict_path, HashSet::from(["x".to_string()]))
        .expect("sudachi dictionary")
        .with_lexicon(lexicon)
        .with_master_readings(&i.master)
        .with_standard(&i.standard)
        .with_frequency(i.ranks.clone())
        .with_reader_frequency(i.reader_ranks.clone())
        .with_preferred_readings(i.preferences.clone())
        .with_conjugatable(i.conjugatable.clone())
        .with_names(i.names.clone())
}

/// One block per line: the sentence, then the identities the ledger would get
/// from it. An identity the master does not list is marked `?`, since those are
/// what the vocabulary count is blind to.
pub fn snapshot(dict_path: &Path, corpus: &[String], i: &Inputs) -> String {
    let tk = tokenizer(dict_path, i);
    let lexicon: HashSet<String> = i.master.iter().map(|(t, _)| t.clone()).collect();
    let words = MasterWords::new(lexicon, &i.master);

    let mut body = String::new();
    let (mut counted, mut unlisted) = (0usize, 0usize);
    let mut distinct: HashSet<(String, String)> = HashSet::new();
    for line in corpus {
        body.push_str(line);
        body.push('\n');
        body.push_str("   ");
        for t in tk.tokenize(line).expect("tokenize") {
            if !counts_as_word(&t, &words) {
                continue;
            }
            counted += 1;
            let reading = to_hiragana(&t.reading);
            distinct.insert((t.base_form.clone(), reading.clone()));
            let listed = words.lists(&t.base_form, &t.reading);
            if !listed {
                unlisted += 1;
            }
            body.push_str(&format!(
                " {}/{}{}",
                t.base_form,
                reading,
                if listed { "" } else { "?" }
            ));
        }
        body.push_str("\n\n");
    }

    // The header is the summary worth watching: the share of counted tokens the
    // master cannot account for is what a vocabulary estimate is built on.
    format!(
        "# {} lines, {counted} counted tokens, {} distinct identities\n\
         # {unlisted} counted tokens ({:.1}%) are identities the master does not list\n\
         # regenerate: cargo run --example golden -p jp-core --features test-support\n\n{body}",
        corpus.len(),
        distinct.len(),
        100.0 * unlisted as f64 / counted.max(1) as f64,
    )
}
