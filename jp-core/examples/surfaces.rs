//! Recompute `term_surfaces` from a corpus with the production tokenizer, so a
//! change can be reviewed before it is written to the database.
//!
//! ```text
//! cargo run --release --example surfaces -p jp-core -- <master.tsv> <conj.tsv> <anki.tsv> <dict.dic> <lines.txt> <freq.tsv> <prefer.tsv>
//!
//! All seven inputs, because the tokenizer behaves differently without any of
//! them: the frequency ranks and the reading preferences are what let the
//! identity ladder's last steps fire, so a probe missing them reports words as
//! unlisted that production resolves.
//! ```
//!
//! Prints `rank<TAB>identity<TAB>total<TAB>surface<TAB>count<TAB>example line`.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use jp_core::knowledge::dictionaries::PreferredReading;
use jp_core::tokenize::{MasterWords, SudachiTokenizer, Tokenizer, counts_as_word};

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
    let set = |p: &str| -> HashSet<String> {
        std::fs::read_to_string(p)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    };
    let ranks: HashMap<(String, String), i64> = std::fs::read_to_string(&a[6])
        .unwrap()
        .lines()
        .filter_map(|l| {
            let mut it = l.split('\t');
            let term = it.next()?.to_string();
            let reading = jp_core::text::kana::to_hiragana(it.next()?);
            Some(((term, reading), it.next()?.trim().parse().ok()?))
        })
        .collect();
    let prefer: HashMap<String, PreferredReading> = std::fs::read_to_string(&a[7])
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
    let tk = SudachiTokenizer::new(Path::new(&a[4]), set(&a[3]))
        .unwrap()
        .with_lexicon(lexicon.clone())
        .with_master_readings(&entries)
        .with_frequency(ranks)
        .with_preferred_readings(prefer)
        .with_conjugatable(set(&a[2]));
    let words = MasterWords::new(lexicon, &entries);

    let mut totals: HashMap<String, usize> = HashMap::new();
    let mut per: HashMap<(String, String), (usize, String)> = HashMap::new();
    for line in std::fs::read_to_string(&a[5]).unwrap().lines() {
        for t in tk.tokenize(line).unwrap() {
            if !counts_as_word(&t, &words) {
                continue;
            }
            let id = format!("{}/{}", t.base_form, t.reading);
            *totals.entry(id.clone()).or_default() += 1;
            let e = per.entry((id, t.surface)).or_insert((0, line.to_string()));
            e.0 += 1;
        }
    }
    let mut ranked: Vec<_> = totals.iter().collect();
    ranked.sort_by_key(|(id, n)| (std::cmp::Reverse(**n), (*id).clone()));
    for (rank, (id, total)) in ranked.iter().enumerate() {
        let mut rows: Vec<_> = per
            .iter()
            .filter(|((i, _), _)| i == *id)
            .map(|((_, s), (n, l))| (n, s, l))
            .collect();
        rows.sort_by_key(|(n, s, _)| (std::cmp::Reverse(**n), (*s).clone()));
        for (n, s, l) in rows {
            let l: String = l.chars().take(46).collect();
            println!("{}\t{id}\t{total}\t{s}\t{n}\t{l}", rank + 1);
        }
    }
}
