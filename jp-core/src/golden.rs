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

/// Build the tokenizer exactly as `read-stats` ingest does.
pub fn tokenizer(
    dict_path: &Path,
    master: &[(String, String)],
    standard: &[(String, String)],
    ranks: HashMap<(String, String), i64>,
    reader_ranks: HashMap<String, i64>,
    preferences: HashMap<String, PreferredReading>,
    conjugatable: HashSet<String>,
) -> SudachiTokenizer {
    let lexicon: HashSet<String> = master.iter().map(|(t, _)| t.clone()).collect();
    // A non-empty deck is what puts it on the C→B→A path that production runs.
    SudachiTokenizer::new(dict_path, HashSet::from(["x".to_string()]))
        .expect("sudachi dictionary")
        .with_lexicon(lexicon)
        .with_master_readings(master)
        .with_standard(standard)
        .with_frequency(ranks)
        .with_reader_frequency(reader_ranks)
        .with_preferred_readings(preferences)
        .with_conjugatable(conjugatable)
}

/// One block per line: the sentence, then the identities the ledger would get
/// from it. An identity the master does not list is marked `?`, since those are
/// what the vocabulary count is blind to.
pub fn snapshot(
    dict_path: &Path,
    corpus: &[String],
    master: &[(String, String)],
    standard: &[(String, String)],
    ranks: HashMap<(String, String), i64>,
    reader_ranks: HashMap<String, i64>,
    preferences: HashMap<String, PreferredReading>,
    conjugatable: HashSet<String>,
) -> String {
    let tk = tokenizer(
        dict_path,
        master,
        standard,
        ranks,
        reader_ranks,
        preferences,
        conjugatable,
    );
    let lexicon: HashSet<String> = master.iter().map(|(t, _)| t.clone()).collect();
    let words = MasterWords::new(lexicon, master);

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
