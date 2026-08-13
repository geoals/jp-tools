//! The lexeme layer — which of the ledger's rows are the same *word*.
//!
//! `vocabulary` keys on `(headword, reading)` — an orthographic **form**, not a
//! word. That is the right key for the ledger: the tokenizer emits forms, and
//! 零れ落ちる at forty encounters vs こぼれ落ちる at two is real information. But
//! it is the wrong unit to *count*. Three rows for 叔父, 伯父 and おじ are one
//! word, so "I know N words" off a row count overstates N by however many
//! spellings the reading history contained.
//!
//! **Everything here is derived and nothing is stored.** There must be no
//! `redundant` column: a flag written at import time depends on what was already
//! in the ledger, so Anki-then-jiten and jiten-then-Anki would leave different
//! databases. Computed at read time it is a pure function of the known forms, so
//! every import order converges and re-running anything is free.
//!
//! A form's lexeme is, **in this order**:
//!
//! 1. **For a kana-only form, the lexeme of the kanji forms that read it**, when
//!    they all agree on one. つらい collapses into 辛い/つらい; おじ collapses
//!    into 叔父 and 伯父, which share an entry id. そう does not, because 相,
//!    添う, 沿う and 層 are four lexemes and the adverb is a fifth word.
//!
//!    Running this *before* the form's own id is load-bearing: Jitendex lists
//!    からい as its own entry separately from 辛い/からい, and taking the id
//!    first would count them as two words.
//! 2. **The form's own entry id**, when the dictionary resolves it to exactly
//!    one — which makes 零れ落ちる and こぼれ落ちる one word while keeping
//!    辛い/からい and 辛い/つらい apart.
//! 3. **The form itself**, so a term the reference dictionary does not list
//!    counts once on its own. Nothing is dropped for being unresolvable.
//!
//! [`known_lexemes`] answers "how many words do I know"; [`redundant_forms`]
//! answers "which rows should triage stop asking about". Deliberately **not**
//! the same set — see its doc.

use std::collections::{HashMap, HashSet};

use sqlx::Row;

use super::Knowledge;
use super::dictionaries;
use super::vocabulary::{COUNTS_AS_VOCAB, Term};
use crate::text::kana;

/// What a form was collapsed onto. `Seq` is a dictionary entry id; `Form` is
/// a form standing alone because nothing resolved it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Lexeme {
    Seq(i64),
    Form(String, String),
}

impl Lexeme {
    fn of(term: &Term) -> Lexeme {
        Lexeme::Form(term.headword.clone(), term.reading.clone())
    }
}

/// One known form, with whatever the reference dictionary says it is.
struct KnownForm {
    term: Term,
    /// Distinct entry ids the dictionary gives this exact `(term, reading)`.
    /// More than one means the dictionary cannot decide, so it is not used.
    sequences: Vec<i64>,
}

impl KnownForm {
    /// Rule 2: the form's own id, when unambiguous.
    fn own_lexeme(&self) -> Option<Lexeme> {
        match self.sequences.as_slice() {
            [one] => Some(Lexeme::Seq(*one)),
            _ => None,
        }
    }
}

/// Load every `known` form that counts toward the vocabulary scale, with its
/// entry ids.
///
/// `in_master` is the same gate the status counts use. The reference dictionary
/// only *groups* those rows; it never admits new ones.
///
/// The join pins `dictionary_id` and matches on `term`, so it rides
/// `idx_dictionary_entries_lookup` as a SEARCH. Check `EXPLAIN QUERY PLAN` before
/// changing it — the shape that scanned held the write lock for six minutes.
async fn known_forms(k: &Knowledge) -> Result<Vec<KnownForm>, sqlx::Error> {
    let dict = dictionaries::lexeme_dictionary(k.pool()).await?;

    let rows = sqlx::query(&format!(
        "SELECT v.headword, v.reading, de.sequence \
         FROM vocabulary v \
         LEFT JOIN dictionary_entries de \
           ON de.dictionary_id = ? \
          AND de.term = v.headword \
          AND de.reading = CASE WHEN v.reading = '' THEN v.headword ELSE v.reading END \
          AND de.sequence IS NOT NULL \
         WHERE v.status = 'known' AND {COUNTS_AS_VOCAB}"
    ))
    .bind(dict)
    .fetch_all(k.pool())
    .await?;

    // The LEFT JOIN fans a form out into one row per entry id, so regroup.
    let mut by_form: HashMap<(String, String), Vec<i64>> = HashMap::new();
    for r in &rows {
        let key: (String, String) = (r.get("headword"), r.get("reading"));
        let entry = by_form.entry(key).or_default();
        if let Some(seq) = r.get::<Option<i64>, _>("sequence")
            && !entry.contains(&seq)
        {
            entry.push(seq);
        }
    }

    Ok(by_form
        .into_iter()
        .map(|((headword, reading), sequences)| KnownForm {
            term: Term { headword, reading },
            sequences,
        })
        .collect())
}

/// Resolve every known form to a lexeme, applying the three rules in order.
async fn resolve(k: &Knowledge) -> Result<HashMap<Term, Lexeme>, sqlx::Error> {
    let forms = known_forms(k).await?;

    // Rule 1's input: for each reading, the lexemes of the *non-kana* forms
    // that carry it. Only non-kana, because the whole point is to fold a kana
    // spelling into a kanji one — folding kana into kana would just pick a
    // winner between two rows that are already the same string.
    let mut by_reading: HashMap<String, HashSet<Lexeme>> = HashMap::new();
    for f in &forms {
        if kana::is_all_kana(&f.term.headword) {
            continue;
        }
        let lexeme = f.own_lexeme().unwrap_or_else(|| Lexeme::of(&f.term));
        by_reading
            .entry(f.term.display_reading().to_string())
            .or_default()
            .insert(lexeme);
    }

    let mut out = HashMap::new();
    for f in forms {
        let lexeme = if kana::is_all_kana(&f.term.headword) {
            // Rule 1, then 2, then 3.
            by_reading
                .get(&f.term.headword)
                .and_then(|s| match s.len() {
                    // All the kanji spellings that read this agree on one
                    // word, so the kana spelling is that word too.
                    1 => s.iter().next().cloned(),
                    // Several different words share the reading. Collapsing
                    // would assert one of them arbitrarily; standing alone is
                    // the honest answer, and usually the correct one.
                    _ => None,
                })
                .or_else(|| f.own_lexeme())
                .unwrap_or_else(|| Lexeme::of(&f.term))
        } else {
            f.own_lexeme().unwrap_or_else(|| Lexeme::of(&f.term))
        };
        out.insert(f.term, lexeme);
    }
    Ok(out)
}

/// Every known form with the lexeme it collapses onto — the grouping the counts
/// are built from, for a caller that needs the mapping itself and not its size.
pub async fn resolved_forms(k: &Knowledge) -> Result<HashMap<Term, Lexeme>, sqlx::Error> {
    resolve(k).await
}

/// How many distinct **words** are known, as opposed to how many rows say
/// `known`.
///
/// This is the figure "I know N words" should be built on. The difference
/// between it and the row count is pure spelling: alternate kanji forms of one
/// entry, and kana spellings of words known in kanji.
pub async fn known_lexemes(k: &Knowledge) -> Result<i64, sqlx::Error> {
    let resolved = resolve(k).await?;
    let distinct: HashSet<&Lexeme> = resolved.values().collect();
    Ok(distinct.len() as i64)
}

/// Both figures at once, for a caller that wants to show the gap.
pub struct LexemeCount {
    /// Rows with `status = 'known'` and `in_master = 1`.
    pub forms: i64,
    /// Distinct words among them.
    pub lexemes: i64,
}

pub async fn known_counts(k: &Knowledge) -> Result<LexemeCount, sqlx::Error> {
    let resolved = resolve(k).await?;
    let distinct: HashSet<&Lexeme> = resolved.values().collect();
    Ok(LexemeCount {
        forms: resolved.len() as i64,
        lexemes: distinct.len() as i64,
    })
}

/// Forms triage should stop offering, because a known form already covers them.
///
/// **Not the same as sharing a lexeme**: redundancy runs one direction only. A
/// form is covered when its kanji are a *subset* of a known form's — こぼれ落ちる
/// ⊆ 零れ落ちる, and pure kana ⊆ anything. Knowing the harder spelling settles
/// the easier one; the reverse settles nothing about whether 零 can be read.
///
/// Running it both ways would quietly mark unread kanji spellings known.
pub async fn redundant_forms(k: &Knowledge) -> Result<HashSet<Term>, sqlx::Error> {
    let known = resolve(k).await?;

    // Index the known forms by reading: a candidate can only be covered by a
    // spelling of the same reading, so that is the whole search space.
    let mut by_reading: HashMap<&str, Vec<&Term>> = HashMap::new();
    for term in known.keys() {
        by_reading
            .entry(term.display_reading())
            .or_default()
            .push(term);
    }

    let rows = sqlx::query(&format!(
        "SELECT headword, reading FROM vocabulary WHERE status != 'known' AND {COUNTS_AS_VOCAB}"
    ))
    .fetch_all(k.pool())
    .await?;

    let mut out = HashSet::new();
    for r in &rows {
        let term = Term {
            headword: r.get("headword"),
            reading: r.get("reading"),
        };
        let covered = by_reading
            .get(term.display_reading())
            .is_some_and(|spellings| spellings.iter().any(|kt| covers(kt, &term)));
        if covered {
            out.insert(term);
        }
    }
    Ok(out)
}

/// Whether knowing `known` settles `candidate`: same reading, and every kanji
/// in the candidate also appears in the known spelling.
fn covers(known: &Term, candidate: &Term) -> bool {
    if known.headword == candidate.headword {
        return false;
    }
    let known_kanji: HashSet<char> = known.headword.chars().filter(|c| is_kanji(*c)).collect();
    candidate
        .headword
        .chars()
        .filter(|c| is_kanji(*c))
        .all(|c| known_kanji.contains(&c))
}

fn is_kanji(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::vocabulary::{self, Status};

    async fn temp() -> Knowledge {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("jp-core-lexeme-{nanos}.db"));
        Knowledge::open(path.to_str().unwrap()).await.unwrap()
    }

    /// Stand up a reference dictionary carrying entry ids, and a master
    /// dictionary carrying none — the real shape: Jitendex groups, Sankoku
    /// admits.
    async fn with_dict(k: &Knowledge, entries: &[(&str, &str, i64)]) {
        sqlx::query("INSERT INTO dictionaries (id, title, source_path, role) VALUES (1, 'ref', 'r.zip', 'reference')")
            .execute(k.pool()).await.unwrap();
        for (term, reading, seq) in entries {
            sqlx::query(
                "INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json, sequence) \
                 VALUES (1, ?, ?, '[]', ?)",
            )
            .bind(term).bind(reading).bind(seq)
            .execute(k.pool()).await.unwrap();
        }
    }

    /// Mark known and count as vocabulary. `in_master` is set directly: these
    /// tests are about grouping, not about the wordhood gate.
    async fn know(k: &Knowledge, headword: &str, reading: &str) {
        let term = Term::new(headword, reading);
        vocabulary::set_status(k, &term, Status::Known, 1.0)
            .await
            .unwrap();
        sqlx::query("UPDATE vocabulary SET in_master = 1 WHERE headword = ? AND reading = ?")
            .bind(&term.headword)
            .bind(&term.reading)
            .execute(k.pool())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn two_spellings_of_one_entry_are_one_word() {
        let k = temp().await;
        with_dict(
            &k,
            &[
                ("零れ落ちる", "こぼれおちる", 2002270),
                ("こぼれ落ちる", "こぼれおちる", 2002270),
            ],
        )
        .await;
        know(&k, "零れ落ちる", "こぼれおちる").await;
        know(&k, "こぼれ落ちる", "こぼれおちる").await;

        let c = known_counts(&k).await.unwrap();
        assert_eq!(
            c.forms, 2,
            "both rows still exist and keep their own counts"
        );
        assert_eq!(c.lexemes, 1, "but they are one word");
    }

    #[tokio::test]
    async fn a_homograph_stays_two_words() {
        let k = temp().await;
        with_dict(
            &k,
            &[("辛い", "からい", 1365850), ("辛い", "つらい", 1365860)],
        )
        .await;
        know(&k, "辛い", "からい").await;
        know(&k, "辛い", "つらい").await;

        assert_eq!(known_lexemes(&k).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn a_kana_spelling_folds_into_the_kanji_ones_that_agree() {
        let k = temp().await;
        // 叔父 and 伯父 are one entry; 小父 is a different word that happens to
        // read the same, and is not known.
        with_dict(
            &k,
            &[
                ("叔父", "おじ", 1607070),
                ("伯父", "おじ", 1607070),
                ("小父", "おじ", 2140540),
            ],
        )
        .await;
        know(&k, "叔父", "おじ").await;
        know(&k, "伯父", "おじ").await;
        know(&k, "おじ", "おじ").await;

        let c = known_counts(&k).await.unwrap();
        assert_eq!(c.forms, 3);
        assert_eq!(c.lexemes, 1, "叔父 / 伯父 / おじ are one word");
    }

    #[tokio::test]
    async fn a_kana_form_beats_its_own_entry_id() {
        let k = temp().await;
        // Jitendex really does list からい separately from 辛い/からい. Taking
        // the form's own id first would count them as two words.
        with_dict(
            &k,
            &[("辛い", "からい", 1365850), ("からい", "からい", 1609860)],
        )
        .await;
        know(&k, "辛い", "からい").await;
        know(&k, "からい", "からい").await;

        assert_eq!(known_lexemes(&k).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn a_reading_shared_by_different_words_does_not_collapse() {
        let k = temp().await;
        with_dict(
            &k,
            &[
                ("層", "そう", 1400700),
                ("沿う", "そう", 1385600),
                ("そう", "そう", 2137720),
            ],
        )
        .await;
        know(&k, "層", "そう").await;
        know(&k, "沿う", "そう").await;
        know(&k, "そう", "そう").await;

        assert_eq!(
            known_lexemes(&k).await.unwrap(),
            3,
            "the adverb そう is its own word, not a spelling of 層"
        );
    }

    #[tokio::test]
    async fn a_term_the_reference_dictionary_misses_still_counts_once() {
        let k = temp().await;
        with_dict(&k, &[("叔父", "おじ", 1607070)]).await;
        know(&k, "叔父", "おじ").await;
        know(&k, "腹を探る", "はらをさぐる").await;

        assert_eq!(
            known_lexemes(&k).await.unwrap(),
            2,
            "nothing is dropped for being unresolvable"
        );
    }

    #[tokio::test]
    async fn the_count_does_not_depend_on_import_order() {
        let entries: &[(&str, &str, i64)] = &[
            ("叔父", "おじ", 1607070),
            ("伯父", "おじ", 1607070),
            ("辛い", "つらい", 1365860),
        ];
        let forward = ["叔父", "伯父", "おじ", "辛い", "つらい"];
        let reverse = ["つらい", "辛い", "おじ", "伯父", "叔父"];

        let mut results = Vec::new();
        for order in [forward, reverse] {
            let k = temp().await;
            with_dict(&k, entries).await;
            for headword in order {
                let reading = match headword {
                    "辛い" => "つらい",
                    "叔父" | "伯父" => "おじ",
                    other => other,
                };
                know(&k, headword, reading).await;
            }
            results.push(known_lexemes(&k).await.unwrap());
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(results[0], 2, "おじ-the-uncle and 辛い/つらい");
    }

    #[tokio::test]
    async fn redundancy_runs_from_the_harder_spelling_to_the_easier() {
        let k = temp().await;
        with_dict(
            &k,
            &[
                ("零れ落ちる", "こぼれおちる", 2002270),
                ("こぼれ落ちる", "こぼれおちる", 2002270),
            ],
        )
        .await;
        know(&k, "零れ落ちる", "こぼれおちる").await;
        // Not known, and in the queue.
        let easier = Term::new("こぼれ落ちる", "こぼれおちる");
        vocabulary::set_status(&k, &easier, Status::New, 1.0)
            .await
            .unwrap();
        sqlx::query("UPDATE vocabulary SET in_master = 1 WHERE headword = 'こぼれ落ちる'")
            .execute(k.pool())
            .await
            .unwrap();

        assert!(
            redundant_forms(&k).await.unwrap().contains(&easier),
            "knowing 零れ落ちる settles こぼれ落ちる"
        );
    }

    #[tokio::test]
    async fn redundancy_does_not_run_the_other_way() {
        let k = temp().await;
        with_dict(
            &k,
            &[
                ("零れ落ちる", "こぼれおちる", 2002270),
                ("こぼれ落ちる", "こぼれおちる", 2002270),
            ],
        )
        .await;
        know(&k, "こぼれ落ちる", "こぼれおちる").await;
        let harder = Term::new("零れ落ちる", "こぼれおちる");
        vocabulary::set_status(&k, &harder, Status::New, 1.0)
            .await
            .unwrap();
        sqlx::query("UPDATE vocabulary SET in_master = 1 WHERE headword = '零れ落ちる'")
            .execute(k.pool())
            .await
            .unwrap();

        assert!(
            !redundant_forms(&k).await.unwrap().contains(&harder),
            "knowing こぼれ落ちる says nothing about reading 零"
        );
    }
}
