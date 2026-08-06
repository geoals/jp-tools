//! Ad-hoc probe: what would a wider join rule do to the corpus?
//!
//! The tokenizer joins adjacent tokens only when the **master** dictionary
//! lists the result (`recompose`), so an expression only Jitendex holds —
//! しびれを切らす, 経年劣化 — is left as its parts. Widening that to "any
//! dictionary" is the obvious fix and was rejected once already, on measurement:
//! longest-match against the line froze じゃない, しまった and 分からない into
//! single ledger rows.
//!
//! This runs the *narrow* version of the wider rule over every line read, so the
//! list can be looked at before anything writes it:
//!
//! - runs of 2–4 adjacent tokens, at every position,
//! - the last token as written and again in canonical form (切らし → 切らす),
//!   which is what a literal match cannot do,
//! - no auxiliary, symbol or punctuation anywhere in the run — that is the
//!   filter じゃない and しまった fail,
//! - at most one particle, and never at either edge,
//! - the result listed by some loaded dictionary.
//!
//! ```text
//! cargo run --release --example joins -p jp-core -- <knowledge.db> <sudachi.dic>
//! ```

use std::collections::{HashMap, HashSet};
use std::path::Path;

use jp_core::knowledge::Knowledge;
use jp_core::knowledge::dictionaries;
use jp_core::tokenize::{SudachiTokenizer, Token, Tokenizer, is_content_word};

/// Longest run considered, in tokens. Four reaches 目 + を + 丸く + する.
const MAX_RUN: usize = 4;

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    if let Err(e) = rt.block_on(run()) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let a: Vec<String> = std::env::args().collect();
    let (db, dic) = (a[1].clone(), a[2].clone());

    let knowledge = Knowledge::open(&db).await.map_err(|e| e.to_string())?;
    let pool = knowledge.pool();

    // The same five inputs the reader and the ingest build their tokenizer
    // with. A tokenizer missing any of them is a second pipeline and answers
    // differently, which would make every number here about nothing.
    let entries = dictionaries::master_entries(pool)
        .await
        .map_err(|e| e.to_string())?;
    let lexicon: HashSet<String> = entries.iter().map(|(t, _)| t.clone()).collect();
    let conjugatable = dictionaries::master_conjugatable(pool)
        .await
        .map_err(|e| e.to_string())?;
    let ranks = match dictionaries::by_title(pool, "BCCWJ")
        .await
        .map_err(|e| e.to_string())?
    {
        Some(d) => {
            let terms = jp_core::tokenize::ambiguous_headwords(&entries);
            dictionaries::frequency_ranks(pool, d.id, &terms)
                .await
                .map_err(|e| e.to_string())?
        }
        None => HashMap::new(),
    };
    let preferred = match (
        dictionaries::master(pool).await.map_err(|e| e.to_string())?,
        dictionaries::by_title(pool, "Jitendex")
            .await
            .map_err(|e| e.to_string())?,
        dictionaries::by_title(pool, "BCCWJ")
            .await
            .map_err(|e| e.to_string())?,
    ) {
        (Some(m), Some(j), Some(b)) => dictionaries::preferred_readings(pool, m.id, j.id, b.id)
            .await
            .map_err(|e| e.to_string())?,
        _ => HashMap::new(),
    };
    let mined: HashSet<String> = sqlx::query_scalar("SELECT DISTINCT vocab FROM anki_notes")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .collect();

    let all_headwords = dictionaries::get_all_headwords(pool)
        .await
        .map_err(|e| e.to_string())?;
    eprintln!(
        "{} master headwords, {} headwords in all dictionaries, {} mined",
        lexicon.len(),
        all_headwords.len(),
        mined.len()
    );

    let tk = SudachiTokenizer::new(Path::new(&dic), mined)
        .map_err(|e| e.to_string())?
        .with_lexicon(lexicon.clone())
        .with_master_readings(&entries)
        .with_frequency(ranks)
        .with_preferred_readings(preferred)
        .with_conjugatable(conjugatable);

    let lines: Vec<String> =
        sqlx::query_scalar("SELECT text FROM lines WHERE discarded = 0 ORDER BY id")
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
    eprintln!("{} lines", lines.len());

    // term -> (occurrences, needed deinflection, the run's parts, in master)
    let mut hits: HashMap<String, (usize, bool, String)> = HashMap::new();
    let mut lines_touched = 0usize;
    for text in &lines {
        let Ok(tokens) = tk.tokenize(text) else {
            continue;
        };
        let mut hit_here = false;
        for start in 0..tokens.len() {
            for len in 2..=MAX_RUN.min(tokens.len() - start) {
                let run = &tokens[start..start + len];
                if !joinable(run) {
                    continue;
                }
                for (term, deinflected) in forms(run) {
                    if term.chars().count() < 3 || !all_headwords.contains(&term) {
                        continue;
                    }
                    let parts = run
                        .iter()
                        .map(|t| t.surface.as_str())
                        .collect::<Vec<_>>()
                        .join("+");
                    let e = hits.entry(term).or_insert((0, deinflected, parts));
                    e.0 += 1;
                    e.1 |= deinflected;
                    hit_here = true;
                }
            }
        }
        lines_touched += usize::from(hit_here);
    }

    let mut rows: Vec<(String, usize, bool, String)> = hits
        .into_iter()
        .map(|(term, (n, deinflected, parts))| (term, n, deinflected, parts))
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let total: usize = rows.iter().map(|r| r.1).sum();
    let new_to_master = rows.iter().filter(|r| !lexicon.contains(&r.0)).count();
    eprintln!(
        "\n{} distinct joins, {total} occurrences, on {lines_touched} of {} lines",
        rows.len(),
        lines.len()
    );
    eprintln!(
        "{new_to_master} of them the master does not list — the ones this rule would add",
    );

    println!("count\tterm\tmaster\tdeinflected\tparts");
    for (term, n, deinflected, parts) in &rows {
        println!(
            "{n}\t{term}\t{}\t{}\t{parts}",
            if lexicon.contains(term) { "master" } else { "-" },
            if *deinflected { "deinflected" } else { "-" },
        );
    }
    Ok(())
}

/// Is this run the shape a joined expression has?
///
/// The filter that separates 目を丸くする from じゃない: an auxiliary anywhere
/// in the run disqualifies it, a particle is allowed once and only inside, and
/// everything else has to be a content word. A stem is allowed only as the last
/// token, where the canonical form puts it back.
fn joinable(run: &[Token]) -> bool {
    let mut particles = 0;
    for (i, t) in run.iter().enumerate() {
        let last = i + 1 == run.len();
        match t.pos.as_str() {
            "助動詞" | "補助記号" | "記号" | "空白" | "接続詞" | "感動詞" | "フィラー" => {
                return false;
            }
            "助詞" => {
                if i == 0 || last {
                    return false;
                }
                particles += 1;
            }
            "接頭辞" | "接尾辞" => {}
            pos if is_content_word(pos) => {}
            _ => return false,
        }
        if t.proper_noun || (t.inflected && !last) {
            return false;
        }
    }
    particles <= 1
}

/// The run as written, and again with its last token in canonical form.
fn forms(run: &[Token]) -> Vec<(String, bool)> {
    let head: String = run[..run.len() - 1].iter().map(|t| t.surface.as_str()).collect();
    let last = &run[run.len() - 1];
    let written = format!("{head}{}", last.surface);
    let canonical = format!("{head}{}", last.base_form);
    if written == canonical {
        vec![(written, false)]
    } else {
        vec![(written, false), (canonical, true)]
    }
}
