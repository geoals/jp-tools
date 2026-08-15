//! Every join the tokenizer actually made, and the lines it made them on.
//!
//! `recompose` builds a token out of adjacent ones whenever the master lists
//! the result, and the master is a learner's dictionary — so it lists the
//! grammar points. それは, ものを, からに and ために are headwords, and the join
//! cannot tell the grammar point from the plain word plus a particle that
//! happens to spell it. [`NEVER_JOIN`](jp_core::tokenize) is the list of the
//! ones judged so far; this is how the next one is found.
//!
//! Sorted by how often each result was built. A run whose parts are a content
//! word and a particle is marked `*`, since that is the shape the defect takes.
//!
//! ```text
//! cargo run --release --example joined -p jp-core -- <knowledge.db> <sudachi.dic> [min-count]
//! ```

use std::collections::HashMap;

use jp_core::knowledge::Knowledge;
use jp_core::tokenize::trace::{Step, Verdict};
use jp_core::tokenize::{SudachiTokenizer, Tokenizer, is_content_word};

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(run());
}

/// One built expression: how often, out of which parts, and where to look.
#[derive(Default)]
struct Built {
    count: usize,
    parts: Vec<String>,
    lines: Vec<String>,
}

async fn run() {
    let a: Vec<String> = std::env::args().collect();
    let min: usize = a.get(3).and_then(|n| n.parse().ok()).unwrap_or(1);
    let k = Knowledge::open(&a[1]).await.unwrap();

    let lines: Vec<String> =
        sqlx::query_scalar("SELECT text FROM lines WHERE discarded = 0 ORDER BY id")
            .fetch_all(k.pool())
            .await
            .unwrap();
    let p = jp_core::highlight::pipeline(&k, &a[2]).await.unwrap();

    let mut built: HashMap<String, Built> = HashMap::new();
    for text in &lines {
        let Ok((_, steps)) = p.tokenizer.explain(text) else {
            continue;
        };
        for step in steps {
            let Step::Join {
                parts,
                verdict: Verdict::Joined { term, .. },
            } = step
            else {
                continue;
            };
            let e = built.entry(term).or_default();
            e.count += 1;
            if e.parts.is_empty() {
                e.parts = parts;
            }
            if e.lines.len() < 3 {
                e.lines.push(text.clone());
            }
        }
    }

    let mut rows: Vec<(&String, &Built)> = built.iter().filter(|(_, b)| b.count >= min).collect();
    rows.sort_by_key(|(term, b)| (std::cmp::Reverse(b.count), (*term).clone()));
    println!("{} distinct expressions built", rows.len());
    for (term, b) in rows {
        let mark = if word_plus_particle(&b.parts, &p.tokenizer) {
            "*"
        } else {
            " "
        };
        println!("\n{mark} {:5} {term}  ←  {}", b.count, b.parts.join(" + "));
        for l in &b.lines {
            println!("        {l}");
        }
    }
}

/// The shape the defect takes: a content word, then nothing but grammar. それは
/// and ものを are this; 振り + 返る and 気 + に + なる are not.
fn word_plus_particle(parts: &[String], tk: &SudachiTokenizer) -> bool {
    let mut classes = parts.iter().map(|p| -> String {
        tk.tokenize(p)
            .ok()
            .and_then(|t| t.first().map(|t| t.pos.clone()))
            .unwrap_or_default()
    });
    let Some(head) = classes.next() else {
        return false;
    };
    is_content_word(&head) && classes.all(|pos| !is_content_word(&pos))
}
