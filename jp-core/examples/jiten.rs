//! Run jiten.moe's segmentation corpus through our tokenizer and show where the
//! two disagree.
//!
//! ```text
//! cargo run --example jiten -p jp-core -- \
//!     ~/.local/share/jp-tools/knowledge.db cases.tsv system_full.dic > report.txt
//! ```
//!
//! The corpus is `jp-core/tests/jiten/cases.tsv`: `sentence<TAB>tok tok tok`,
//! extracted from
//! `Jiten.Tests/MorphologicalAnalyserTests.cs` (github.com/Sirush/Jiten), which
//! in turn started from ichiran's `tests.lisp`. Jiten runs Sudachi too, so its
//! expectations are a second opinion on the same engine.
//!
//! It is not a pass/fail gate: jiten's "word" is a vocabulary-list unit and
//! keeps expressions whole (雨が降りそう, タバコをやめる), while ours is a ledger
//! key. Only the diff is interesting, and only some of it is a bug.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use jp_core::knowledge::Knowledge;
use jp_core::knowledge::dictionaries;
use jp_core::tokenize::Tokenizer;

/// Both sides drop punctuation, so compare on the words alone.
fn is_punctuation(s: &str) -> bool {
    s.chars().all(|c| {
        c.is_whitespace()
            || c.is_ascii_punctuation()
            || ('\u{3000}'..='\u{303f}').contains(&c)
            || (('\u{ff01}'..='\u{ff20}').contains(&c) && !('\u{ff10}'..='\u{ff19}').contains(&c))
            || c == '…'
            || c == '―'
            || c == '─'
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let db = &args[1];
    let cases_src = std::fs::read_to_string(&args[2]).unwrap();
    let dict_path = Path::new(&args[3]);

    let cases: Vec<(String, Vec<String>)> = cases_src
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut it = l.splitn(2, '\t');
            let text = it.next().unwrap().to_string();
            let toks = it
                .next()
                .unwrap_or("")
                .split_whitespace()
                .map(str::to_string)
                .collect();
            (text, toks)
        })
        .collect();

    let k = Knowledge::open(db).await.unwrap();
    let pool = k.pool();
    let master = dictionaries::master_entries(pool).await.unwrap();
    let bccwj = dictionaries::by_title(pool, "BCCWJ")
        .await
        .unwrap()
        .unwrap();
    let jitendex = dictionaries::by_title(pool, "Jitendex")
        .await
        .unwrap()
        .unwrap();
    let master_dict = dictionaries::master(pool).await.unwrap().unwrap();
    let ambiguous = jp_core::tokenize::ambiguous_headwords(&master);
    let ranks: HashMap<(String, String), i64> = dictionaries::frequency_ranks(pool, bccwj.id, &ambiguous)
        .await
        .unwrap();
    let prefs = dictionaries::preferred_readings(pool, master_dict.id, jitendex.id, bccwj.id)
        .await
        .unwrap();
    let conjugatable: HashSet<String> = dictionaries::master_conjugatable(pool).await.unwrap();

    let tk = jp_core::golden::tokenizer(dict_path, &master, ranks, prefs, conjugatable);

    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut sections: HashMap<&str, String> = HashMap::new();
    for (text, theirs) in &cases {
        let ours: Vec<String> = tk
            .tokenize(text)
            .expect("tokenize")
            .into_iter()
            .map(|t| t.surface)
            .filter(|s| !is_punctuation(s))
            .collect();
        let kind = classify(&ours, theirs);
        *counts.entry(kind).or_default() += 1;
        if kind == "same" {
            continue;
        }
        sections.entry(kind).or_default().push_str(&format!(
            "{text}\n  jiten: {}\n  ours : {}\n\n",
            theirs.join(" | "),
            ours.join(" | ")
        ));
    }

    let order = ["cross", "unalignable", "we-split", "we-merge", "mixed"];
    let mut head = format!("# {} cases\n", cases.len());
    for kind in ["same"].iter().chain(order.iter()) {
        head.push_str(&format!(
            "#   {kind:12} {}\n",
            counts.get(kind).copied().unwrap_or(0)
        ));
    }
    head.push_str(
        "#\n# cross = the two cut the same span in incompatible places: the bug class.\n\
         # we-split / we-merge = one side's boundaries are a subset of the other's,\n\
         # which is mostly the convention difference (jiten keeps expressions whole).\n\
         # unalignable = the two token streams do not spell the same text.\n\n",
    );

    print!("{head}");
    for kind in order {
        if let Some(body) = sections.get(kind) {
            println!("{:=^70}\n", format!(" {kind} "));
            print!("{body}");
        }
    }
}

/// Where the two segmentations stand relative to each other, on boundaries
/// rather than tokens: a boundary either side has and the other lacks is a
/// merge, and a boundary neither contains is the two cutting the same span in
/// different places — the only case that is a candidate bug on both sides.
fn classify(ours: &[String], theirs: &[String]) -> &'static str {
    if ours == theirs {
        return "same";
    }
    let (a, b) = (cuts(ours), cuts(theirs));
    if ours.concat() != theirs.concat() {
        return "unalignable";
    }
    match (a.is_subset(&b), b.is_subset(&a)) {
        (true, _) => "we-merge",
        (_, true) => "we-split",
        _ => "cross",
    }
}

/// Character offsets where a token ends, the final one excluded — it is shared
/// by every segmentation and says nothing.
fn cuts(tokens: &[String]) -> HashSet<usize> {
    let mut at = 0;
    let mut out = HashSet::new();
    for t in tokens {
        at += t.chars().count();
        out.insert(at);
    }
    out.remove(&at);
    out
}
