//! Mechanical audit of the identities the corpus yields — no judgement in it.
//!
//! ```text
//! cargo run --release --example audit -p jp-core --features test-support -- \
//!     <knowledge.db> <system_full.dic> [class]
//! ```
//!
//! Two tests, both decidable from the token alone, and between them they are
//! the two ways an identity can be an assertion nobody read:
//!
//! - `added-kanji` — the identity spells a kanji the surface does not contain.
//!   とうもろこし keyed as 玉蜀黍, いびき as 鼾, ご祝儀 as 御祝儀. Legitimate
//!   folding (いう → 言う) looks identical, so the rank is printed beside it:
//!   the question this is really asking is how rare the spelling being asserted
//!   is.
//! - `kana-to-kanji` — an all-hiragana surface that took a kanji identity, with
//!   the rank of what it took. This is where the false positives live: 弥 off
//!   いや, 対置 off たいち.
//!
//! `AUDIT_JOINS=1` audits the other pass instead: every run of tokens
//! recomposition merged, by what it built and how rare that word is. A join is
//! the mirror defect of a bad identity — there Sudachi put a boundary wrong,
//! here it was right and the join removed a correct one — and the same test
//! applies, since a rank-119,268 word built out of two particles was not read
//! either.
//!
//! Output is TSV, sorted by occurrences, so a threshold can be chosen by
//! reading the distribution rather than guessed.

use std::collections::HashMap;
use std::path::Path;

use jp_core::knowledge::Knowledge;
use jp_core::knowledge::dictionaries;
use jp_core::text::kana;
use jp_core::text::kanji::is_kanji;
use jp_core::tokenize::MasterWords;

#[derive(Default)]
struct Row {
    count: usize,
    example: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let db = &args[1];
    let dict_path = Path::new(&args[2]);
    let only = args.get(3).cloned();

    let k = Knowledge::open(db).await.unwrap();
    let pool = k.pool();

    let master = dictionaries::master_entries(pool).await.unwrap();
    let standard = dictionaries::standard_entries(pool).await.unwrap();
    let ambiguous = jp_core::tokenize::ambiguous_headwords(&master);
    let bccwj = dictionaries::by_title(pool, "BCCWJ")
        .await
        .unwrap()
        .unwrap();
    let ranks = dictionaries::frequency_ranks(pool, bccwj.id, &ambiguous)
        .await
        .unwrap();
    let jitendex = dictionaries::by_title(pool, "Jitendex")
        .await
        .unwrap()
        .unwrap();
    let master_dict = dictionaries::master(pool).await.unwrap().unwrap();
    let prefs = dictionaries::preferred_readings(pool, master_dict.id, jitendex.id, bccwj.id)
        .await
        .unwrap();
    let conjugatable = dictionaries::master_conjugatable(pool).await.unwrap();

    // The reader-facing rank, which is the one that says whether a spelling is
    // one anybody writes — see `READER_FREQUENCY` in jp-core's CLAUDE.md.
    let reader_freq = dictionaries::by_title(pool, dictionaries::READER_FREQUENCY)
        .await
        .unwrap()
        .unwrap();
    let jiten: HashMap<String, i64> =
        sqlx::query_as::<_, (String, i64)>(
            "select term, min(frequency) from dictionary_frequency where dictionary_id = ? group by term",
        )
        .bind(reader_freq.id)
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .collect();

    // **`discarded = 0`, as every other reader of this table does.** 106 lines
    // of mojibake from one badly-hooked session carry 21% of the corpus's
    // characters, and they were cleared from the reader long ago — measuring
    // over them describes a tokenizer nobody runs.
    let lines: Vec<String> = sqlx::query_scalar(
        "select text from lines where text is not null and discarded = 0 order by id",
    )
    .fetch_all(pool)
    .await
    .unwrap();

    // `AUDIT_GUARD=off` builds the tokenizer without the reader ranks, which is
    // the only thing the short-kana guard needs — so the two runs differ in that
    // rule and nothing else, and their diff is the rule's whole blast radius.
    let lexicon: std::collections::HashSet<String> =
        master.iter().map(|(t, _)| t.clone()).collect();
    let words = MasterWords::new(lexicon, &master);
    let guarded = std::env::var("AUDIT_GUARD").as_deref() != Ok("off");
    // `AUDIT_CAST=off` builds it without the cast, the same way, so the two
    // runs differ in the name pass and nothing else.
    let names: std::collections::HashSet<String> =
        if std::env::var("AUDIT_CAST").as_deref() == Ok("off") {
            Default::default()
        } else {
            jp_core::knowledge::work_names::all(&k)
                .await
                .unwrap()
                .into_iter()
                .collect()
        };
    let tk = jp_core::golden::tokenizer(
        dict_path,
        &jp_core::golden::Inputs {
            master: master.clone(),
            standard,
            ranks,
            reader_ranks: if guarded {
                jiten.clone()
            } else {
                HashMap::new()
            },
            preferences: prefs,
            conjugatable,
            names,
        },
    );

    // `AUDIT_TEXT="…"` explains one line the way `#tokenize` does, without a
    // running service — which is how a defect gets checked while a session is
    // in progress.
    if let Ok(text) = std::env::var("AUDIT_TEXT") {
        for line in text.split('\n') {
            let (tokens, steps) = tk.explain(line).unwrap();
            println!("== {line}");
            for t in &tokens {
                println!(
                    "   {:<10} -> {:<12} {:<12} {:<8} word={} name={}",
                    t.surface,
                    t.base_form,
                    kana::to_hiragana(&t.reading),
                    t.pos,
                    jp_core::tokenize::counts_as_word(t, &words),
                    t.proper_noun
                );
            }
            for step in steps {
                println!("   . {}", serde_json::to_string(&step).unwrap());
            }
        }
        return;
    }

    // `AUDIT_NAMES=1`: tokens Sudachi calls 固有名詞 that the master lists as
    // an ordinary headword — the words the name gate drops before it consults
    // the ledger, and 断腸の思い is the one that was noticed.
    // `AUDIT_SAMPLE=N`: N counted tokens drawn uniformly from the corpus, with
    // the line each came from — the input to a judged accuracy estimate. Drawn
    // by occurrence, not by type, because that is what the ledger accumulates.
    //
    // `AUDIT_TYPES=N` draws distinct identities instead, one vote each, which is
    // the shape the two audits at the end of PARSE-DEFECTS.md used.
    let sample = std::env::var("AUDIT_SAMPLE")
        .ok()
        .or_else(|| std::env::var("AUDIT_TYPES").ok())
        .and_then(|n| n.parse::<usize>().ok());
    if let Some(n) = sample {
        let by_type = std::env::var("AUDIT_TYPES").is_ok();
        let mut seen: Vec<(String, String, String)> = Vec::new();
        let mut types: HashMap<(String, String), (String, String)> = HashMap::new();
        for line in &lines {
            let Ok(tokens) = jp_core::tokenize::Tokenizer::tokenize(&tk, line) else {
                continue;
            };
            for t in tokens {
                if !jp_core::tokenize::counts_as_word(&t, &words) || t.proper_noun {
                    continue;
                }
                let id = format!("{}/{}", t.base_form, kana::to_hiragana(&t.reading));
                let clean = line.replace(['\t', '\n'], " ");
                if by_type {
                    types
                        .entry((t.base_form.clone(), t.reading.clone()))
                        .or_insert((id, clean));
                } else {
                    seen.push((t.surface.clone(), id, clean));
                }
            }
        }
        if by_type {
            let mut keys: Vec<_> = types.into_iter().collect();
            keys.sort_by(|a, b| a.0.cmp(&b.0));
            seen = keys
                .into_iter()
                .map(|(_, (id, line))| (String::new(), id, line))
                .collect();
        }
        // A fixed multiplier walk rather than an RNG dependency: coprime stride
        // over the population, so the draw is spread and reproducible.
        let total = seen.len();
        eprintln!("population {total}, drawing {n}");
        let stride = 7919usize;
        for i in 0..n.min(total) {
            let (surface, id, line) = &seen[(i * stride + 13) % total];
            println!("{}\t{surface}\t{id}\t{line}", i + 1);
        }
        return;
    }

    if std::env::var("AUDIT_NAMES").is_ok() {
        let mut found: HashMap<String, Row> = HashMap::new();
        for line in &lines {
            let Ok(tokens) = jp_core::tokenize::Tokenizer::tokenize(&tk, line) else {
                continue;
            };
            for t in tokens {
                if !t.proper_noun || !words.lists(&t.base_form, &t.reading) {
                    continue;
                }
                let row = found
                    .entry(format!("{}/{}", t.base_form, kana::to_hiragana(&t.reading)))
                    .or_default();
                row.count += 1;
                if row.example.is_empty() {
                    row.example = line.replace(['\t', '\n'], " ");
                }
            }
        }
        let mut rows: Vec<_> = found.into_iter().collect();
        rows.sort_by_key(|(k, r)| (std::cmp::Reverse(r.count), k.clone()));
        eprintln!(
            "{} distinct, {} occurrences dropped as names though the master lists them",
            rows.len(),
            rows.iter().map(|(_, r)| r.count).sum::<usize>()
        );
        for (term, row) in rows {
            println!("{}\t{term}\t{}", row.count, row.example);
        }
        return;
    }

    if std::env::var("AUDIT_JOINS").is_ok() {
        return joins(&tk, &lines, &jiten);
    }

    let mut found: HashMap<(String, String, String, String, String), Row> = HashMap::new();
    let mut counted = 0usize;
    for line in &lines {
        let Ok((_, steps)) = tk.explain(line) else {
            continue;
        };
        for step in steps {
            let jp_core::tokenize::trace::Step::Identity {
                surface,
                headword,
                reading,
                rule,
                ..
            } = step
            else {
                continue;
            };
            counted += 1;
            if headword == surface {
                continue;
            }
            // The identity puts a kanji on the page the surface has none of.
            if !headword
                .chars()
                .any(|c| is_kanji(c) && !surface.contains(c))
            {
                continue;
            }
            let class = if kana::is_all_hiragana(&surface) {
                "kana-to-kanji"
            } else {
                "added-kanji"
            };
            if only.as_deref().is_some_and(|c| c != class) {
                continue;
            }
            let rank = jiten
                .get(&headword)
                .map(|r| r.to_string())
                .unwrap_or_else(|| "-".into());
            let row = found
                .entry((
                    class.to_string(),
                    surface.clone(),
                    format!("{headword}/{}", kana::to_hiragana(&reading)),
                    rank,
                    rung(rule).to_string(),
                ))
                .or_default();
            row.count += 1;
            if row.example.is_empty() {
                row.example = line.replace(['\t', '\n'], " ");
            }
        }
    }

    let mut rows: Vec<_> = found.into_iter().collect();
    rows.sort_by_key(|((class, surface, id, _, _), r)| {
        (
            class.clone(),
            std::cmp::Reverse(r.count),
            surface.clone(),
            id.clone(),
        )
    });
    let total: usize = rows.iter().map(|(_, r)| r.count).sum();
    eprintln!(
        "{} lines, {counted} identities, {total} flagged ({:.2}%), {} distinct",
        lines.len(),
        100.0 * total as f64 / counted.max(1) as f64,
        rows.len()
    );
    println!("class\tn\tsurface\tidentity\trank\trung\texample");
    for ((class, surface, id, rank, rung), row) in rows {
        println!(
            "{class}\t{}\t{surface}\t{id}\t{rank}\t{rung}\t{}",
            row.count, row.example
        );
    }
}

/// The ladder rung, shortened to something a column can hold.
fn rung(rule: &str) -> &'static str {
    match rule {
        r if r.starts_with("Exact match") => "exact",
        r if r.starts_with("Matched by spelling") => "spelling",
        r if r.starts_with("Matched by reading only") => "reading-only",
        r if r.starts_with("Obsolete reading") => "preferred",
        r if r.starts_with("Not in master dictionary: kept") => "kept-as-written",
        r if r.starts_with("Not in master dictionary") => "sudachi",
        r if r.starts_with("Single-mora") => "one-mora",
        r if r.starts_with("Invalid word start") => "bad-onset",
        _ => "other",
    }
}

/// Every join recomposition made, by what it built. The parts are kept because
/// they are the evidence: よ + って spelling よって is the defect, and よって
/// arriving whole from Sudachi is not.
fn joins(tk: &jp_core::tokenize::SudachiTokenizer, lines: &[String], jiten: &HashMap<String, i64>) {
    use jp_core::tokenize::trace::{Step, Verdict};
    let mut found: HashMap<(String, String, String, String), Row> = HashMap::new();
    let mut total = 0usize;
    for line in lines {
        let Ok((_, steps)) = tk.explain(line) else {
            continue;
        };
        for step in steps {
            let Step::Join { parts, verdict } = step else {
                continue;
            };
            let Verdict::Joined {
                term,
                reading,
                signal,
            } = verdict
            else {
                continue;
            };
            total += 1;
            let rank = jiten
                .get(&term)
                .map(|r| r.to_string())
                .unwrap_or_else(|| "-".into());
            let row = found
                .entry((
                    format!("{term}/{}", kana::to_hiragana(&reading)),
                    parts.join("+"),
                    rank,
                    signal.to_string(),
                ))
                .or_default();
            row.count += 1;
            if row.example.is_empty() {
                row.example = line.replace(['\t', '\n'], " ");
            }
        }
    }
    let mut rows: Vec<_> = found.into_iter().collect();
    rows.sort_by_key(|(k, r)| (std::cmp::Reverse(r.count), k.clone()));
    eprintln!("{total} joins, {} distinct", rows.len());
    println!("n\tterm\tparts\trank\tsignal\texample");
    for ((term, parts, rank, signal), row) in rows {
        println!(
            "{}\t{term}\t{parts}\t{rank}\t{signal}\t{}",
            row.count, row.example
        );
    }
}
