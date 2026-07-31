//! Read-only: what a rebuild would do to the ledger's counts, without writing
//! a row. Tokenizes the whole history under the current gate and diffs the
//! terms it produces against the terms `vocabulary` holds now.
//!
//! Throwaway. Delete once the number has been read.

use std::collections::{HashMap, HashSet};

use jp_core::knowledge::vocabulary::Term;
use jp_core::knowledge::{Knowledge, dictionaries};
use jp_core::tokenize::{
    MasterWords, SudachiTokenizer, Token, Tokenizer, counts_as_word, is_content_word,
};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = std::env::var("JP_TOOLS_KNOWLEDGE_DB_PATH")?;
    let k = Knowledge::open(&db).await?;
    let lexicon: HashSet<String> = dictionaries::master_headwords(k.pool()).await?;
    let readings = dictionaries::master_entries(k.pool()).await?;
    let anki: Vec<(String,)> = sqlx::query_as("SELECT vocab FROM anki_notes")
        .fetch_all(k.pool())
        .await?;
    let anki: HashSet<String> = anki.into_iter().map(|(v,)| v).collect();
    let lines: Vec<(String,)> =
        sqlx::query_as("SELECT text FROM lines WHERE discarded = 0 ORDER BY id")
            .fetch_all(k.pool())
            .await?;
    let sessions: Vec<(String,)> =
        sqlx::query_as("SELECT content FROM manual_sessions WHERE content != ''")
            .fetch_all(k.pool())
            .await?;

    let dict = std::env::var("JP_TOOLS_SUDACHI_DICT_PATH")?;
    let master = MasterWords::new(lexicon.clone(), &readings);
    let tk = SudachiTokenizer::new(std::path::Path::new(&dict), anki)?
        .with_lexicon(lexicon.clone())
        .with_master_readings(&readings);

    // The pass, exactly as `Harvest` runs it: the name verdict is per term over
    // the whole history, so proper-noun occurrences are tallied and judged at
    // the end rather than filtered as they go.
    let mut counts: HashMap<Term, i64> = HashMap::new();
    let mut proper: HashMap<Term, (i64, i64)> = HashMap::new();
    let mut affix_only: HashSet<Term> = HashSet::new();
    let mut fold = |tokens: Vec<Token>, counts: &mut HashMap<Term, i64>| {
        for t in tokens {
            if !counts_as_word(&t, &master) {
                continue;
            }
            let admitted_by_affix_rule = !is_content_word(&t.pos);
            let term = Term::new(t.base_form, &t.reading);
            let seen = proper.entry(term.clone()).or_default();
            seen.0 += i64::from(t.proper_noun);
            seen.1 += 1;
            if admitted_by_affix_rule {
                affix_only.insert(term.clone());
            }
            *counts.entry(term).or_default() += 1;
        }
    };
    for (text,) in &lines {
        if let Ok(tokens) = tk.tokenize(text) {
            fold(tokens, &mut counts);
        }
    }
    for (content,) in &sessions {
        for sentence in jp_core::text::sentences::split_sentences(content) {
            if let Ok(tokens) = tk.tokenize(&sentence) {
                fold(tokens, &mut counts);
            }
        }
    }
    // A name is not vocabulary, and the verdict is the majority of a term's own
    // occurrences over the whole pass.
    counts.retain(|term, _| {
        let (named, seen) = proper.get(term).copied().unwrap_or((0, 1));
        named * 2 <= seen
    });

    // The ledger as it stands.
    let rows: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT headword, reading, status, in_master, COALESCE(promoted, 0) FROM vocabulary",
    )
    .fetch_all(k.pool())
    .await?;
    let existing: HashMap<Term, (String, bool)> = rows
        .iter()
        .map(|(hw, rd, st, im, pr)| {
            (
                Term {
                    headword: hw.clone(),
                    reading: rd.clone(),
                },
                (st.clone(), *im == 1 || *pr == 1),
            )
        })
        .collect();

    let master_pairs: HashSet<(String, String)> = readings
        .iter()
        .map(|(t, r)| (t.clone(), jp_core::text::kana::to_hiragana(r)))
        .collect();
    let counts_as_vocab = |t: &Term| {
        if let Some((_, v)) = existing.get(t) {
            return *v;
        }
        // A new row's in_master is set by refresh_dictionary_flags, which asks
        // the same question this does.
        if jp_core::text::kana::is_all_kana(&t.headword) {
            lexicon.contains(&t.headword)
        } else {
            master_pairs.contains(&(t.headword.clone(), t.reading.clone()))
        }
    };

    let now_total = existing.len();
    let now_vocab = existing.values().filter(|(_, v)| *v).count();
    let now_known = existing
        .values()
        .filter(|(s, v)| *v && (s == "known"))
        .count();

    let new_terms: Vec<&Term> = counts
        .keys()
        .filter(|t| !existing.contains_key(t))
        .collect();
    let new_vocab = new_terms.iter().filter(|t| counts_as_vocab(t)).count();
    let new_from_affix = new_terms
        .iter()
        .filter(|t| affix_only.contains(**t) && counts_as_vocab(t))
        .count();

    // prune_untouched: rows this pass no longer produces, minus the judged and
    // the mined, which are spared.
    let pruned: Vec<&Term> = existing
        .iter()
        .filter(|(t, (s, _))| !counts.contains_key(*t) && s == "new")
        .map(|(t, _)| t)
        .collect();
    let pruned_vocab = pruned.iter().filter(|t| counts_as_vocab(t)).count();

    println!(
        "ledger now:        {now_total} rows, {now_vocab} count as vocabulary, {now_known} known"
    );
    println!("the pass produces: {} terms", counts.len());
    println!(
        "  new rows:        {} ({} count as vocabulary, of which {} arrive via the affix rule)",
        new_terms.len(),
        new_vocab,
        new_from_affix
    );
    println!(
        "  pruned rows:     {} unjudged rows the pass no longer produces ({} counted as vocabulary)",
        pruned.len(),
        pruned_vocab
    );
    println!(
        "\nafter rebuild:     {} rows, {} count as vocabulary, {now_known} known (status is never rewritten)",
        now_total + new_terms.len() - pruned.len(),
        now_vocab + new_vocab - pruned_vocab
    );

    let mut affix_new: Vec<(&Term, i64)> = new_terms
        .iter()
        .filter(|t| affix_only.contains(**t))
        .map(|t| (*t, counts[*t]))
        .collect();
    affix_new.sort_by_key(|(_, n)| -*n);
    println!("\nthe affix rows that would be added, by encounters:");
    for (t, n) in affix_new.iter().take(25) {
        println!("{n:>6}  {}/{}", t.headword, t.reading);
    }
    Ok(())
}
