//! How often the reading-only fallback has to guess between two words.
//!
//! That step is the weakest one in identity resolution: nothing is left but the
//! sound, and the corpus rank picks the winner. This counts the corpus tokens
//! that reach it and how many candidates each had left after the conjugation
//! filter, which is what decides whether the guess is worth disambiguating with
//! anything smarter.
//!
//! ```text
//! JP_TOOLS_SUDACHI_DICT_PATH=$PWD/system_full.dic \
//!   cargo run --release --example homophones -p jp-core --features test-support
//! ```

use std::collections::{HashMap, HashSet};
use std::path::Path;

use jp_core::knowledge::dictionaries;
use jp_core::text::kana::to_hiragana;
use jp_core::tokenize::ambiguous_headwords;
use jp_core::tokenize::trace::Step;

const READING_ONLY: &str = "Matched by reading only";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dict_path = std::env::var("JP_TOOLS_SUDACHI_DICT_PATH")?;
    let db = std::env::var("HOME")? + "/.local/share/jp-tools/knowledge.db";
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{db}?mode=ro")).await?;

    let master = dictionaries::master_entries(&pool).await?;
    let standard = dictionaries::standard_entries(&pool).await?;
    let conjugatable = dictionaries::master_conjugatable(&pool).await?;
    let ranks = match dictionaries::by_title(&pool, "BCCWJ").await? {
        Some(b) => {
            dictionaries::frequency_ranks(&pool, b.id, &ambiguous_headwords(&master)).await?
        }
        None => HashMap::new(),
    };
    let preferences = match (
        dictionaries::master(&pool).await?,
        dictionaries::by_title(&pool, "Jitendex").await?,
        dictionaries::by_title(&pool, "BCCWJ").await?,
    ) {
        (Some(m), Some(j), Some(b)) => {
            dictionaries::preferred_readings(&pool, m.id, j.id, b.id).await?
        }
        _ => HashMap::new(),
    };

    let lines: Vec<String> = sqlx::query_scalar("SELECT text FROM lines")
        .fetch_all(&pool)
        .await?;

    // Every master headword that reads a given way, so a token that reached the
    // fallback can be asked how many words it was choosing between.
    let mut by_reading: HashMap<String, HashSet<String>> = HashMap::new();
    for (term, reading) in &master {
        by_reading
            .entry(to_hiragana(reading))
            .or_default()
            .insert(term.clone());
    }

    let tk = jp_core::golden::tokenizer(
        Path::new(&dict_path),
        &master,
        &standard,
        ranks,
        HashMap::new(),
        preferences,
        conjugatable.clone(),
    );

    let mut total = 0usize;
    let mut ambiguous = 0usize;
    let mut per_word: HashMap<(String, String, String), usize> = HashMap::new();
    for line in &lines {
        let Ok((_, steps)) = tk.explain(line) else {
            continue;
        };
        for step in &steps {
            let Step::Identity {
                surface,
                headword,
                reading,
                rule,
                ..
            } = step
            else {
                continue;
            };
            if !rule.starts_with(READING_ONLY) {
                continue;
            }
            total += 1;
            let choices = by_reading
                .get(&to_hiragana(reading))
                .map(|terms| terms.iter().filter(|t| conjugatable.contains(*t)).count())
                .unwrap_or(0);
            if choices > 1 {
                ambiguous += 1;
                *per_word
                    .entry((surface.clone(), to_hiragana(reading), headword.clone()))
                    .or_default() += 1;
            }
        }
    }

    println!("{} lines", lines.len());
    println!("{total} tokens resolved by reading alone");
    println!("{ambiguous} of those had more than one verb to choose between");
    let mut ranked: Vec<_> = per_word.into_iter().collect();
    ranked.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for ((surface, reading, headword), n) in ranked.iter().take(40) {
        let others: Vec<&str> = by_reading
            .get(reading)
            .map(|terms| {
                terms
                    .iter()
                    .filter(|t| conjugatable.contains(*t))
                    .map(String::as_str)
                    .collect()
            })
            .unwrap_or_default();
        println!("{n:>5}  {surface} -> {headword}   [{}]", others.join(" "));
    }
    Ok(())
}
