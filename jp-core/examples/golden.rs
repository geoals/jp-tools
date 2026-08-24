//! Regenerate the golden tokenizer fixtures in `tests/golden/`.
//!
//! ```text
//! cargo run --example golden -p jp-core -- \
//!     ~/.local/share/kotodex/knowledge.db lines.txt system_full.dic
//! ```
//!
//! where `lines.txt` is the read corpus
//! (`sqlite3 knowledge.db "select text from lines"`). Everything the tokenizer
//! needs is written out beside the snapshot, so the test itself never touches a
//! database — a golden test that reads live data is not golden.
//!
//! Run this only when a change is *meant* to move the output, and read the diff.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use jp_core::knowledge::Knowledge;
use jp_core::knowledge::dictionaries;

/// Lines long enough to exercise the machinery and written with real kanji.
/// A kana-only line is mostly interjections and teaches the snapshot nothing.
const LINES: usize = 250;
const MIN_LEN: usize = 10;
const MAX_LEN: usize = 40;
const MIN_KANJI: usize = 3;

fn is_kanji_line(line: &str) -> bool {
    let len = line.chars().count();
    let kanji = line
        .chars()
        .filter(|c| jp_core::text::kanji::is_kanji(*c))
        .count();
    (MIN_LEN..=MAX_LEN).contains(&len)
        && kanji >= MIN_KANJI
        && !line.chars().any(|c| c.is_ascii_alphanumeric())
}

/// Deterministic sample: every nth line of the eligible ones. No RNG, so
/// regenerating on the same corpus gives the same fixture.
///
/// **Only for a fixture that does not exist yet.** The step is `len / 250`, so
/// it moves as the corpus grows and a resample would swap the 250 sentences
/// under the snapshot — leaving a diff no one can read against the rule that
/// was changed. Delete `corpus.txt` to draw a new sample deliberately.
fn sample(lines: Vec<&str>) -> Vec<String> {
    let step = (lines.len() / LINES).max(1);
    lines
        .into_iter()
        .step_by(step)
        .take(LINES)
        .map(str::to_string)
        .collect()
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let db = &args[1];
    let corpus_src = std::fs::read_to_string(&args[2]).unwrap();
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    std::fs::create_dir_all(&out).unwrap();

    let existing = std::fs::read_to_string(out.join("corpus.txt")).unwrap_or_default();
    let corpus: Vec<String> = if existing.trim().is_empty() {
        sample(
            corpus_src
                .lines()
                .map(str::trim)
                .filter(|l| is_kanji_line(l))
                .collect(),
        )
    } else {
        existing.lines().map(str::to_string).collect()
    };
    let text: String = corpus.concat();
    std::fs::write(out.join("corpus.txt"), corpus.join("\n") + "\n").unwrap();

    let k = Knowledge::open(db).await.unwrap();
    let pool = k.pool();
    let entries = dictionaries::master_entries(pool).await.unwrap();

    // Only the master entries these lines can reach: the ones written in the
    // text, the ones *read* in it (伺う is reached through うかがう and is not a
    // substring of anything), and anything sharing a reading with those, since
    // that is what the reading fallback arbitrates between.
    let mut readings: HashSet<&str> = HashSet::new();
    for (term, reading) in &entries {
        if text.contains(term.as_str()) || text.contains(reading.as_str()) {
            readings.insert(reading);
        }
    }
    let mut kept: Vec<&(String, String)> = entries
        .iter()
        .filter(|(term, reading)| {
            text.contains(term.as_str()) || readings.contains(reading.as_str())
        })
        .collect();
    kept.sort();
    kept.dedup();
    let master: Vec<(String, String)> = kept.iter().map(|(t, r)| (t.clone(), r.clone())).collect();
    std::fs::write(
        out.join("master.tsv"),
        master
            .iter()
            .map(|(t, r)| format!("{t}\t{r}\n"))
            .collect::<String>(),
    )
    .unwrap();

    let lexicon: HashSet<String> = master.iter().map(|(t, _)| t.clone()).collect();
    let bccwj = dictionaries::by_title(pool, "BCCWJ")
        .await
        .unwrap()
        .unwrap();
    let ambiguous = jp_core::tokenize::ambiguous_headwords(&master);
    let ranks = dictionaries::frequency_ranks(pool, bccwj.id, &ambiguous)
        .await
        .unwrap();
    write_map(out.join("frequency.tsv"), &ranks);

    // The reader-facing rank of every master headword these lines can reach.
    // Narrowed to `lexicon` and no further: the short-kana guard reads an absent
    // term as rare, so a fixture missing a ranked headword would answer
    // differently from production rather than merely less often.
    let reader = dictionaries::reader_frequency(pool).await.unwrap().unwrap();
    let reader_ranks: HashMap<String, i64> = sqlx::query_as::<_, (String, i64)>(
        "SELECT term, MIN(frequency) FROM dictionary_frequency
         WHERE dictionary_id = ? GROUP BY term",
    )
    .bind(reader.id)
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .filter(|(term, _)| lexicon.contains(term))
    .collect();
    let mut reader_rows: Vec<String> = reader_ranks
        .iter()
        .map(|(t, n)| format!("{t}\t{n}\n"))
        .collect();
    reader_rows.sort();
    std::fs::write(out.join("reader_ranks.tsv"), reader_rows.concat()).unwrap();

    let jitendex = dictionaries::by_title(pool, "Jitendex")
        .await
        .unwrap()
        .unwrap();
    let master_dict = dictionaries::master(pool).await.unwrap().unwrap();
    let all_prefs = dictionaries::preferred_readings(pool, master_dict.id, jitendex.id, bccwj.id)
        .await
        .unwrap();
    let mut prefs: Vec<String> = all_prefs
        .iter()
        .filter(|(term, _)| lexicon.contains(*term))
        .map(|(term, p)| {
            let mut ok: Vec<&str> = p.acceptable.iter().map(String::as_str).collect();
            ok.sort();
            format!("{term}\t{}\t{}\n", p.preferred, ok.join(","))
        })
        .collect();
    prefs.sort();
    std::fs::write(out.join("preferences.tsv"), prefs.concat()).unwrap();

    // The segmentation authority beside the master, narrowed to what these
    // lines can reach the same way master.tsv is. A fixture that skipped it
    // would be a third pipeline, and this one exists to be production.
    let standard: Vec<(String, String)> = dictionaries::standard_entries(pool)
        .await
        .unwrap()
        .into_iter()
        .filter(|(term, reading)| text.contains(term.as_str()) || text.contains(reading.as_str()))
        .collect();
    let mut standard_rows: Vec<String> = standard
        .iter()
        .map(|(t, r)| format!("{t}\t{r}\n"))
        .collect();
    standard_rows.sort();
    standard_rows.dedup();
    std::fs::write(out.join("standard.tsv"), standard_rows.concat()).unwrap();

    let conjugatable: HashSet<String> = dictionaries::master_conjugatable(pool)
        .await
        .unwrap()
        .into_iter()
        .filter(|t| lexicon.contains(t))
        .collect();
    let mut rows: Vec<&String> = conjugatable.iter().collect();
    rows.sort();
    std::fs::write(
        out.join("conjugatable.txt"),
        rows.iter().map(|t| format!("{t}\n")).collect::<String>(),
    )
    .unwrap();

    // The cast of every work read, which is what the name pass consults. The
    // whole list, not the part the corpus reaches: it is 62 strings, and a
    // filtered one would hide the name that a later sample happens to contain.
    let names: HashSet<String> = jp_core::knowledge::work_names::all(&k)
        .await
        .unwrap()
        .into_iter()
        .collect();
    let mut name_rows: Vec<&String> = names.iter().collect();
    name_rows.sort();
    std::fs::write(
        out.join("names.txt"),
        name_rows
            .iter()
            .map(|n| format!("{n}\n"))
            .collect::<String>(),
    )
    .unwrap();

    // The snapshot itself, produced by the same code the test will run.
    let dict_path = Path::new(&args[3]);
    let snapshot = jp_core::golden::snapshot(
        dict_path,
        &corpus,
        &jp_core::golden::Inputs {
            master: master.clone(),
            standard,
            ranks,
            reader_ranks,
            preferences: all_prefs,
            conjugatable,
            names,
        },
    );
    std::fs::write(out.join("identities.txt"), snapshot).unwrap();
    eprintln!(
        "wrote {} lines, {} master entries to {}",
        corpus.len(),
        master.len(),
        out.display()
    );
}

fn write_map(path: PathBuf, map: &HashMap<(String, String), i64>) {
    let mut rows: Vec<String> = map
        .iter()
        .map(|((t, r), n)| format!("{t}\t{r}\t{n}\n"))
        .collect();
    rows.sort();
    std::fs::write(path, rows.concat()).unwrap();
}
