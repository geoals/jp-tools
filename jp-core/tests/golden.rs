//! The regression net: 250 real sentences, and the identities the ledger gets
//! from them.
//!
//! Every rule in the tokenizer trades one class of mistake for another, and the
//! individual tests beside this one only check the cases someone thought of.
//! This one shows the whole blast radius of a change — including the sentences
//! nobody had in mind, which is where the last four regressions were found.
//!
//! ```text
//! JP_TOOLS_SUDACHI_DICT_PATH=$PWD/../system_full.dic \
//!   cargo test -p jp-core --features test-support --test golden -- --ignored
//! ```
//!
//! When it fails, **read the diff before regenerating**: it is the review, not
//! an obstacle. `cargo run --example golden -p jp-core --features test-support`
//! rewrites the fixture once the change is the one you meant.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use jp_core::knowledge::dictionaries::PreferredReading;

fn tsv(name: &str) -> Vec<Vec<String>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect()
}

#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn the_identities_a_corpus_yields_have_not_moved() {
    let dict_path = std::env::var("JP_TOOLS_SUDACHI_DICT_PATH")
        .expect("JP_TOOLS_SUDACHI_DICT_PATH must be set");

    let corpus: Vec<String> = include_str!("golden/corpus.txt")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect();
    let master: Vec<(String, String)> = tsv("master.tsv")
        .into_iter()
        .map(|r| (r[0].clone(), r.get(1).cloned().unwrap_or_default()))
        .collect();
    let ranks: HashMap<String, i64> = tsv("frequency.tsv")
        .into_iter()
        .filter_map(|r| Some((r[0].clone(), r.get(1)?.parse().ok()?)))
        .collect();
    let preferences: HashMap<String, PreferredReading> = tsv("preferences.tsv")
        .into_iter()
        .map(|r| {
            (
                r[0].clone(),
                PreferredReading {
                    preferred: r[1].clone(),
                    acceptable: r[2].split(',').map(str::to_string).collect(),
                },
            )
        })
        .collect();

    let conjugatable: HashSet<String> = include_str!("golden/conjugatable.txt")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect();

    let produced = jp_core::golden::snapshot(
        Path::new(&dict_path),
        &corpus,
        &master,
        ranks,
        preferences,
        conjugatable,
    );
    let expected = include_str!("golden/identities.txt");
    if produced == expected {
        return;
    }

    // Show the sentences that changed rather than the whole file, and write the
    // new output somewhere it can be diffed properly.
    let scratch = std::env::temp_dir().join("jp-core-golden-actual.txt");
    std::fs::write(&scratch, &produced).unwrap();
    let changed: Vec<String> = produced
        .lines()
        .zip(expected.lines())
        .filter(|(a, b)| a != b)
        .take(30)
        .map(|(a, b)| format!("  - {b}\n  + {a}"))
        .collect();
    panic!(
        "the tokenizer's output over the golden corpus changed.\n\
         Read these, and regenerate only if they are the change you meant:\n\n{}\n\n\
         full output written to {}\n",
        changed.join("\n"),
        scratch.display()
    );
}
