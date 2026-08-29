//! The dictionary cache: terms, readings, pitch accents and frequency ranks,
//! imported from Yomitan zips and queried by every tool that needs to know
//! whether a string is a word.
//!
//! The dictionaries **cannot be pooled** into one "is it a word" set, because
//! they disagree about what a word is: a bilingual dictionary gives phrases and
//! every orthographic variant a headword of their own where a monolingual lists
//! them under one, so most of Jitendex is absent from Sankoku.
//!
//! Hence [`Role`], and two thresholds over the same data:
//!
//! - **wordhood gate** — lenient: any dictionary will do.
//! - **vocabulary denominator** — strict: the master only.
//!
//! Adding a dictionary must never move the denominator, which is what the role
//! is for: a new import is `reference` until someone says otherwise.

use std::collections::{HashMap, HashSet};

use sqlx::{Row, SqlitePool};

use crate::dictionary::{DictionaryEntry, PitchEntry};

/// Every headword any dictionary lists, roles ignored.
pub async fn get_all_headwords(pool: &SqlitePool) -> Result<HashSet<String>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT DISTINCT term FROM dictionary_entries")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(term,)| term).collect())
}

/// The master dictionary's headwords — the tokenizer's lexicon.
pub async fn master_headwords(pool: &SqlitePool) -> Result<HashSet<String>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT de.term FROM dictionary_entries de \
         JOIN dictionaries d ON d.id = de.dictionary_id WHERE d.role = 'master'",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(term,)| term).collect())
}

/// Every headword, and every reading, of the dictionaries whose listing makes a
/// term a word — the roles [`VocabRow::is_word`] accepts.
///
/// The lenient half of the two thresholds above, for a caller that has to ask
/// the wordhood question of a term the ledger has no row for and so cannot ask
/// `is_word` itself. The readings come back beside the terms because that rule
/// has a kana half: a ledger key with no reading *is* kana, and a dictionary
/// listing it as the reading of a kanji headword is the same evidence of
/// wordhood — see [`refresh_dictionary_flags`], which this mirrors.
///
/// **Every role that lists words at all**, which is what "lenient: any
/// dictionary" has to mean — the frequency and pitch lists are out because they
/// hold ranks and accents rather than headwords. 明鏡 and 小学館 are `standard`
/// because they decide segmentation, and a dictionary trusted to say where a word
/// *ends* has to be trusted to say it is a word: a term only they list needs its
/// span and its popup like any other, 聞きかじり among them.
///
/// [`VocabRow::is_word`]: crate::knowledge::vocabulary::VocabRow::is_word
/// [`refresh_dictionary_flags`]: crate::knowledge::vocabulary::refresh_dictionary_flags
pub async fn wordhood_entries(
    pool: &SqlitePool,
) -> Result<(HashSet<String>, HashSet<String>), sqlx::Error> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        // Roles as a subquery, not a join: the join gives the planner no way to
        // restrict by dictionary before it reads, so it walks the whole entries
        // table and filters afterwards. Resolving the ids first turns it into a
        // search of four dictionaries. Only worth it where the role set covers
        // most of the table — the narrower queries below read the same either
        // way, and a join says what they mean more plainly.
        "SELECT DISTINCT term, reading FROM dictionary_entries \
         WHERE dictionary_id IN \
         (SELECT id FROM dictionaries WHERE role IN ('master', 'name', 'reference', 'standard'))",
    )
    .fetch_all(pool)
    .await?;
    let mut terms = HashSet::new();
    let mut readings = HashSet::new();
    for (term, reading) in rows {
        terms.insert(term);
        if !reading.is_empty() {
            readings.insert(reading);
        }
    }
    Ok((terms, readings))
}

/// The `(headword, reading)` pairs of every dictionary that decides
/// segmentation *beside* the master — see [`Role::Standard`]. Same shape as
/// [`master_entries`], and deliberately a separate query: the caller has to
/// hand these to the tokenizer as the weaker authority they are.
pub async fn standard_entries(pool: &SqlitePool) -> Result<Vec<(String, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT DISTINCT de.term, de.reading FROM dictionary_entries de \
         JOIN dictionaries d ON d.id = de.dictionary_id \
         WHERE d.role = 'standard' AND de.reading != ''",
    )
    .fetch_all(pool)
    .await
}

/// The master dictionary's `(headword, reading)` pairs, for recomposing the
/// compounds Sudachi's own lexicon splits. See
/// `SudachiTokenizer::with_master_readings`, which is what decides how to treat
/// a reading that names more than one headword.
pub async fn master_entries(pool: &SqlitePool) -> Result<Vec<(String, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT DISTINCT de.term, de.reading FROM dictionary_entries de \
         JOIN dictionaries d ON d.id = de.dictionary_id \
         WHERE d.role = 'master' AND de.reading != ''",
    )
    .fetch_all(pool)
    .await
}

/// The master headwords the dictionary itself calls conjugatable lemmas.
///
/// The only thing that separates 許す from 許せ, or 慣れる from 汝: all four are
/// Sankoku headwords, and an inflected verb can only ever *be* one of the two
/// that conjugate. See `SudachiTokenizer::with_conjugatable`.
pub async fn master_conjugatable(pool: &SqlitePool) -> Result<HashSet<String>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as(
        // Subquery for the same reason as `wordhood_entries`, and here because
        // `rules` is in no index: restricting to the master first is what keeps
        // that from being a row fetch per entry in every dictionary.
        "SELECT DISTINCT term FROM dictionary_entries \
         WHERE dictionary_id IN (SELECT id FROM dictionaries WHERE role = 'master') \
         AND rules != ''",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(term,)| term).collect())
}

pub async fn find_dictionary(
    pool: &SqlitePool,
    source_path: &str,
) -> Result<Option<(i64, String)>, sqlx::Error> {
    let row = sqlx::query("SELECT id, title FROM dictionaries WHERE source_path = ?")
        .bind(source_path)
        .fetch_optional(pool)
        .await?;

    Ok(row.map(|r| (r.get("id"), r.get("title"))))
}

/// Insert a dictionary and all its entries in one transaction, so an interrupted
/// import leaves nothing partial behind.
pub async fn import_dictionary(
    pool: &SqlitePool,
    title: &str,
    source_path: &str,
    entries: &[DictionaryEntry],
) -> Result<i64, sqlx::Error> {
    let mut tx = pool.begin().await?;

    // `meta_checked` from the start: a fresh import parses the whole zip, meta
    // banks included, so there is nothing left for the backfill to look for.
    let row = sqlx::query(
        "INSERT INTO dictionaries (title, source_path, meta_checked) VALUES (?, ?, 1) RETURNING id",
    )
    .bind(title)
    .bind(source_path)
    .fetch_one(&mut *tx)
    .await?;
    let dict_id: i64 = row.get("id");

    for entry in entries {
        let definitions_json =
            serde_json::to_string(&entry.definitions).unwrap_or_else(|_| "[]".into());
        sqlx::query(
            "INSERT INTO dictionary_entries (dictionary_id, term, reading, score, definitions_json, sequence, rules) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(dict_id)
        .bind(&entry.term)
        .bind(&entry.reading)
        .bind(entry.score)
        .bind(&definitions_json)
        .bind(entry.sequence)
        .bind(&entry.rules)
        .execute(&mut *tx)
        .await?;
    }

    // A fresh import read the sequences straight off the term banks, so there
    // is nothing for the backfill to do.
    sqlx::query("UPDATE dictionaries SET seq_checked = 1 WHERE id = ?")
        .bind(dict_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(dict_id)
}

/// Re-parse of an already-cached zip: replace its entries in place.
///
/// Not a delete-and-import, because that mints a new id and drops the role back
/// to `reference` — silently taking a dictionary out of segmentation.
pub async fn replace_entries(
    pool: &SqlitePool,
    dictionary_id: i64,
    entries: &[DictionaryEntry],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM dictionary_entries WHERE dictionary_id = ?")
        .bind(dictionary_id)
        .execute(&mut *tx)
        .await?;

    for entry in entries {
        let definitions_json =
            serde_json::to_string(&entry.definitions).unwrap_or_else(|_| "[]".into());
        sqlx::query(
            "INSERT INTO dictionary_entries (dictionary_id, term, reading, score, definitions_json, sequence, rules) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(dictionary_id)
        .bind(&entry.term)
        .bind(&entry.reading)
        .bind(entry.score)
        .bind(&definitions_json)
        .bind(entry.sequence)
        .bind(&entry.rules)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query("UPDATE dictionaries SET seq_checked = 1 WHERE id = ?")
        .bind(dictionary_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Whether this dictionary's zip has already been read for entry ids.
///
/// Separate from "has any sequences": a dictionary that publishes none
/// (Sankoku) must answer `true` once checked, or every startup re-parses a
/// large zip to learn the same nothing.
pub async fn needs_sequence_backfill(
    pool: &SqlitePool,
    dictionary_id: i64,
) -> Result<bool, sqlx::Error> {
    let (checked,): (i64,) = sqlx::query_as("SELECT seq_checked FROM dictionaries WHERE id = ?")
        .bind(dictionary_id)
        .fetch_one(pool)
        .await?;
    Ok(checked == 0)
}

/// Attach entry ids and word classes to a dictionary cached before those
/// columns existed.
///
/// Matched on `(term, reading)` — the same pair the entries were stored
/// under, so this is a straight update and never invents a row. Marks the
/// dictionary checked either way.
///
/// One pass for both, because both come off the same term bank and re-reading a
/// large zip twice to learn two things about the same row is waste.
pub async fn backfill_sequences(
    pool: &SqlitePool,
    dictionary_id: i64,
    entries: &[DictionaryEntry],
) -> Result<u64, sqlx::Error> {
    let mut updated = 0;
    // Committed in batches, not as one transaction over every entry. SQLite has
    // one write lock per database: a single pass over a large dictionary holds it
    // for minutes, which is long enough to fail a line being captured from a live
    // reading session, and long enough to lose the lock and roll the whole pass
    // back.
    for chunk in entries.chunks(2000) {
        let mut tx = pool.begin().await?;
        for entry in chunk {
            if entry.sequence.is_none() && entry.rules.is_empty() {
                continue;
            }
            updated += sqlx::query(
                "UPDATE dictionary_entries \
                    SET sequence = COALESCE(?, sequence), rules = ? \
                  WHERE dictionary_id = ? AND term = ? AND reading = ?",
            )
            .bind(entry.sequence)
            .bind(&entry.rules)
            .bind(dictionary_id)
            .bind(&entry.term)
            .bind(&entry.reading)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        }
        tx.commit().await?;
    }
    // Last, and only once the rows are in: an interrupted run leaves the flag
    // clear and is simply retried.
    sqlx::query("UPDATE dictionaries SET seq_checked = 1 WHERE id = ?")
        .bind(dictionary_id)
        .execute(pool)
        .await?;
    Ok(updated)
}

/// The dictionary whose entry ids define lexemes — the one carrying the most
/// of them.
///
/// Deliberately not the master dictionary: Sankoku is monolingual and publishes
/// no stable ids, so only a reference dictionary can answer "are these two
/// spellings one word?" — even though the master alone decides what counts as
/// vocabulary. The two roles are independent and both are needed.
pub async fn lexeme_dictionary(pool: &SqlitePool) -> Result<Option<i64>, sqlx::Error> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT dictionary_id FROM dictionary_entries WHERE sequence IS NOT NULL \
         GROUP BY dictionary_id ORDER BY COUNT(*) DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.0))
}

/// Every entry id in the lexeme dictionary, mapped to the forms of it that the
/// **master** dictionary also lists.
///
/// The join is what makes an id-keyed import safe: an id names a *word*, but a
/// word is spelt several ways and the ledger keys on spellings. The master join
/// is the vocabulary gate — names and JMdict's multi-word phrases resolve to no
/// master form and import as nothing.
///
/// **Readings are normalized on both sides before matching**, since an entry
/// whose headword is already kana stores the reading either as the headword
/// again or not at all. Without it no kana headword joins.
///
/// **Spellings fan out; readings do not.** An entry groups every way a word has
/// ever been written *and said*, and those are not the same claim. Knowing
/// 回る means knowing 廻る — one word, two spellings, and triage should not ask
/// twice. Knowing 執着 as しゅうちゃく says nothing about しゅうじゃく, which is
/// a reading in its own right; the same for 出入り/ではいり against でいり,
/// 利益/りやく against りえき, 入り口/はいりぐち against いりぐち.
///
/// So a form is admitted when **its reading** is in the entry's best tier.
/// JMdict's priority scores land on tiers — about 200 tagged common, about 99
/// in one list, 0 untagged, negative tagged rare — and a secondary reading sits
/// a whole tier below the primary: しゅうじゃく 98 against しゅうちゃく 200,
/// はいりぐち 96 against いりぐち 200. Tiering by reading rather than by form is
/// what keeps 硬い/固い/堅い (200/199/198, all かたい) together while splitting
/// those apart.
///
/// Negative is excluded outright as well, which is a claim about the *spelling*:
/// 三人 is さんにん at 200 and みたり at -102, 硬い is also 緊い at -103.
///
/// A variant spelling of the primary reading still comes through — 次々/次次,
/// 元々/元元, 不気味/無気味 — and costs nothing, since it collapses onto the
/// same lexeme.
///
/// Returned whole rather than per id: an import asks about most of them.
pub async fn master_forms_by_sequence(
    pool: &SqlitePool,
) -> Result<HashMap<i64, Vec<(String, String)>>, sqlx::Error> {
    let Some(lex) = lexeme_dictionary(pool).await? else {
        return Ok(HashMap::new());
    };
    let Some(master) = master(pool).await? else {
        return Ok(HashMap::new());
    };

    let rows = sqlx::query(
        "WITH tier AS ( \
             SELECT sequence AS seq, \
                    COALESCE(NULLIF(reading, ''), term) AS said, \
                    MAX(CASE WHEN score >= 150 THEN 2 WHEN score >= 0 THEN 1 ELSE 0 END) AS t \
               FROM dictionary_entries \
              WHERE dictionary_id = ? AND sequence IS NOT NULL \
              GROUP BY seq, said), \
          top AS (SELECT seq, MAX(t) AS t FROM tier GROUP BY seq) \
         SELECT DISTINCT j.sequence AS seq, j.term AS term, \
                COALESCE(NULLIF(j.reading, ''), j.term) AS reading \
         FROM dictionary_entries j \
         JOIN dictionary_entries m \
           ON m.dictionary_id = ? AND m.term = j.term \
          AND COALESCE(NULLIF(m.reading, ''), m.term) \
              = COALESCE(NULLIF(j.reading, ''), j.term) \
         JOIN tier ti ON ti.seq = j.sequence \
          AND ti.said = COALESCE(NULLIF(j.reading, ''), j.term) \
         JOIN top tp ON tp.seq = j.sequence AND tp.t = ti.t \
         WHERE j.dictionary_id = ? AND j.sequence IS NOT NULL AND j.score >= 0",
    )
    .bind(lex)
    .bind(master.id)
    .bind(lex)
    .fetch_all(pool)
    .await?;

    let mut out: HashMap<i64, Vec<(String, String)>> = HashMap::new();
    for r in &rows {
        out.entry(r.get("seq"))
            .or_default()
            .push((r.get("term"), r.get("reading")));
    }
    Ok(out)
}

pub async fn lookup_dictionary_entries(
    pool: &SqlitePool,
    dictionary_id: i64,
    term: &str,
) -> Result<Vec<DictionaryEntry>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT term, reading, score, definitions_json, sequence, rules FROM dictionary_entries WHERE dictionary_id = ? AND term = ?",
    )
    .bind(dictionary_id)
    .bind(term)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let json_str: String = r.get("definitions_json");
            let definitions: Vec<String> = serde_json::from_str(&json_str).unwrap_or_default();
            DictionaryEntry {
                term: r.get("term"),
                reading: r.get("reading"),
                score: r.get("score"),
                definitions,
                sequence: r.get("sequence"),
                rules: r.get("rules"),
            }
        })
        .collect())
}

pub async fn insert_pitch_entries(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    dictionary_id: i64,
    entries: &[(String, PitchEntry)],
) -> Result<(), sqlx::Error> {
    for (term, entry) in entries {
        let positions_json =
            serde_json::to_string(&entry.positions).unwrap_or_else(|_| "[]".into());
        sqlx::query(
            "INSERT INTO dictionary_pitch (dictionary_id, term, reading, positions_json) VALUES (?, ?, ?, ?)",
        )
        .bind(dictionary_id)
        .bind(term)
        .bind(&entry.reading)
        .bind(&positions_json)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub async fn lookup_pitch_entries(
    pool: &SqlitePool,
    dictionary_id: i64,
    term: &str,
) -> Result<Vec<PitchEntry>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT reading, positions_json FROM dictionary_pitch WHERE dictionary_id = ? AND term = ?",
    )
    .bind(dictionary_id)
    .bind(term)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let reading: String = r.get("reading");
            let json_str: String = r.get("positions_json");
            let positions: Vec<u32> = serde_json::from_str(&json_str).unwrap_or_default();
            PitchEntry { reading, positions }
        })
        .collect())
}

/// Batched multi-row inserts: a frequency dictionary can hold over a million
/// entries — BCCWJ does — and per-row round-trips make the import take minutes.
pub async fn insert_frequency_entries(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    dictionary_id: i64,
    entries: &[crate::dictionary::FreqEntry],
) -> Result<(), sqlx::Error> {
    // 225 rows × 4 binds = 900 variables, safely under SQLite's limit
    for chunk in entries.chunks(225) {
        let placeholders = vec!["(?, ?, ?, ?)"; chunk.len()].join(",");
        let sql = format!(
            "INSERT INTO dictionary_frequency (dictionary_id, term, reading, frequency) VALUES {placeholders}"
        );
        let mut query = sqlx::query(&sql);
        for e in chunk {
            query = query
                .bind(dictionary_id)
                .bind(&e.term)
                .bind(&e.reading)
                .bind(e.rank);
        }
        query.execute(&mut **tx).await?;
    }
    Ok(())
}

/// A frequency dictionary carries several entries per term — readings, short and
/// long unit words — and the lowest rank is the commonest usage.
pub async fn lookup_frequency(
    pool: &SqlitePool,
    dictionary_id: i64,
    term: &str,
) -> Result<Option<i64>, sqlx::Error> {
    let row: (Option<i64>,) = sqlx::query_as(
        "SELECT MIN(frequency) FROM dictionary_frequency WHERE dictionary_id = ? AND term = ?",
    )
    .bind(dictionary_id)
    .bind(term)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

/// The same rank, for many terms at once.
///
/// The tokenizer needs this to break a reading that names several headwords
/// (うかがう is 伺う and 窺う), and it needs it *before* it starts, so the ranks
/// are fetched in one pass rather than a query per token. Terms with no entry
/// are simply absent — the caller then refuses to arbitrate.
/// **Keyed on the reading as well as the term**, because the question being
/// asked is always "how common is this spelling *read this way*". Ranking the
/// spelling alone answers a different one and answers it wrongly: 一 is rank 23
/// as いち, so いつ resolves to 一 rather than 何時, even though 一/いつ is rank
/// 536,048 against 何時/いつ at 151.
pub async fn frequency_ranks(
    pool: &SqlitePool,
    dictionary_id: i64,
    terms: &[String],
) -> Result<HashMap<(String, String), i64>, sqlx::Error> {
    let mut ranks: HashMap<(String, String), i64> = HashMap::new();
    // SQLite's variable limit is 32k on recent builds and 999 on older ones.
    for chunk in terms.chunks(900) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let sql = format!(
            "SELECT term, reading, MIN(frequency) FROM dictionary_frequency
             WHERE dictionary_id = ? AND term IN ({placeholders}) GROUP BY term, reading"
        );
        let mut q = sqlx::query_as::<_, (String, String, i64)>(&sql).bind(dictionary_id);
        for term in chunk {
            q = q.bind(term);
        }
        for (term, reading, rank) in q.fetch_all(pool).await? {
            let key = (term, crate::text::kana::to_hiragana(&reading));
            ranks
                .entry(key)
                .and_modify(|r| *r = (*r).min(rank))
                .or_insert(rank);
        }
    }
    Ok(ranks)
}

/// How much less popular a reading has to be than the best one before it is
/// treated as the wrong reading rather than a rarer one.
///
/// JMdict's priority scores land on tiers: about 200 (tagged common), about 99
/// (tagged in one list), 0 (untagged), negative (tagged rare or irregular). 150
/// means **only a reading JMdict does not tag at all may be overruled** — one
/// whole tier is not enough, because 街/まち and 身体/からだ score 99 against
/// their 200 and are plainly real readings. 私/わたくし scores 0 against
/// わたし's 200 and is the case this exists for.
///
/// Equal or near-equal readings are never candidates anyway: 何 is なに and なん
/// at 200 each, 一日 is いちにち 200 and ついたち 199.
const POPULARITY_TIER: i64 = 150;

/// Which reading to believe for a headword the master lists several ways.
///
/// Sudachi's cost model is the wrong authority for this and cannot be argued
/// with: it reads a bare 私 as わたくし in every context, and both 私/わたし and
/// 私/わたくし are listed pairs, so no amount of validating the pair moves it.
///
/// Corpus frequency cannot settle it either — BCCWJ is annotated with the same
/// UniDic that Sudachi uses, and duly ranks わたくし (47) ahead of わたし (182).
/// Asking it would confirm the error.
///
/// So the popularity dictionary decides *which* readings are current, being
/// editorially tagged and derived from neither: it scores わたし 200 and
/// わたくし 0. Where several readings are equally current (私 is also あたし at
/// 200) the corpus breaks the tie, which is the one job its ranks are good for.
///
/// Returns, per headword: the reading to prefer, and the readings near enough to
/// the top that they are simply how the word is also read — `acceptable`, which
/// the caller must leave alone.
pub async fn preferred_readings(
    pool: &SqlitePool,
    master_id: i64,
    popularity_id: i64,
    corpus_id: i64,
) -> Result<HashMap<String, PreferredReading>, sqlx::Error> {
    let rows: Vec<(String, String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT m.term, m.reading, \
                COALESCE(MAX(p.score), -9999), \
                (SELECT MIN(f.frequency) FROM dictionary_frequency f \
                  WHERE f.dictionary_id = ? AND f.term = m.term AND f.reading = m.reading) \
           FROM dictionary_entries m \
           LEFT JOIN dictionary_entries p \
             ON p.dictionary_id = ? AND p.term = m.term AND p.reading = m.reading \
          WHERE m.dictionary_id = ? AND m.reading <> '' \
          GROUP BY m.term, m.reading",
    )
    .bind(corpus_id)
    .bind(popularity_id)
    .bind(master_id)
    .fetch_all(pool)
    .await?;

    let mut by_term: HashMap<String, Vec<(String, i64, Option<i64>)>> = HashMap::new();
    for (term, reading, score, rank) in rows {
        by_term
            .entry(term)
            .or_default()
            .push((reading, score, rank));
    }

    let mut out = HashMap::new();
    for (term, readings) in by_term {
        if readings.len() < 2 {
            continue;
        }
        let best = readings.iter().map(|(_, s, _)| *s).max().unwrap_or(0);
        // Nothing here is tagged common, so there is no living reading to
        // correct *to* — only a least-bad one. 何時 is scored なんどき 0 and
        // いつ -101, and preferring なんどき would be the reverse of the truth.
        if best < POPULARITY_TIER {
            continue;
        }
        // A negative score is a tag about the *spelling*, not the reading:
        // JMdict scores 居る/いる at -101 because いる is usually written in kana
        // at all, while 居る/おる sits at 200. Reading that as "おる is the living
        // reading of 居る" inverts the truth, so a reading tagged this way is
        // left alone rather than judged.
        let acceptable: HashSet<String> = readings
            .iter()
            .filter(|(_, s, _)| *s < 0 || best - s < POPULARITY_TIER)
            .map(|(r, _, _)| r.clone())
            .collect();
        // Every reading is current — 何 is なに and なん and the sentence says
        // which. Nothing to prefer.
        if acceptable.len() == readings.len() {
            continue;
        }
        let Some(preferred) = readings
            .iter()
            .filter(|(r, _, _)| acceptable.contains(r))
            // Unranked sorts last: a reading the corpus never saw is not the one
            // to rewrite a word to.
            .min_by_key(|(_, _, rank)| rank.unwrap_or(i64::MAX))
            .map(|(r, _, _)| r.clone())
        else {
            continue;
        };
        out.insert(
            term,
            PreferredReading {
                preferred,
                acceptable,
            },
        );
    }
    Ok(out)
}

/// See [`preferred_readings`].
#[derive(Debug, Clone)]
pub struct PreferredReading {
    pub preferred: String,
    /// Readings that need no correcting, the preferred one included.
    pub acceptable: HashSet<String>,
}

/// Whether this dictionary's zip has already been read for pitch and frequency
/// data.
///
/// Same shape as [`needs_sequence_backfill`], and for the same reason: most
/// dictionaries publish neither, so "has no pitch rows" cannot mean "not read
/// yet" — read that way, every `jp-dict sync` re-parses every zip installed.
pub async fn needs_meta_backfill(
    pool: &SqlitePool,
    dictionary_id: i64,
) -> Result<bool, sqlx::Error> {
    let (checked,): (i64,) = sqlx::query_as("SELECT meta_checked FROM dictionaries WHERE id = ?")
        .bind(dictionary_id)
        .fetch_one(pool)
        .await?;
    Ok(checked == 0)
}

pub async fn mark_meta_checked(pool: &SqlitePool, dictionary_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE dictionaries SET meta_checked = 1 WHERE id = ?")
        .bind(dictionary_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// What a dictionary is *for*, which decides which questions it may answer.
///
/// Stored per row rather than inferred from the title, so importing a new
/// dictionary is a data change and never a code change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The one dictionary whose headword count is a meaningful vocabulary
    /// scale. Exactly one row should hold this.
    Master,
    /// A standard monolingual dictionary beside the master — 明鏡, 小学館.
    /// It decides **segmentation** with the master (意味ありげ is one word
    /// because 明鏡 lists it) and nothing else: not the vocabulary scale, and
    /// not how a word is spelt. Those two stay the master's, because these
    /// dictionaries list the archaic kanji of every function word — それ as
    /// 其れ, この as 此の — and admitting those as spellings respells the
    /// commonest words in the language in kanji no reader ever saw.
    Standard,
    /// A name dictionary: a term in here but not in the master is a name, not
    /// vocabulary.
    Name,
    /// A frequency list. It answers how common a word is — the reader's
    /// underline, the rank pill, the sweep's ordering — and nothing else.
    Frequency,
    /// A pitch-accent dictionary: where the downstep is, for the popup and the
    /// card.
    Pitch,
    /// Everything else — bilingual dictionaries, and anything not yet given a
    /// role. Counts for the wordhood gate, never toward the vocabulary total.
    Reference,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Master => "master",
            Role::Standard => "standard",
            Role::Name => "name",
            Role::Frequency => "frequency",
            Role::Pitch => "pitch",
            Role::Reference => "reference",
        }
    }

    pub fn parse(s: &str) -> Role {
        match s {
            "master" => Role::Master,
            "standard" => Role::Standard,
            "name" => Role::Name,
            "frequency" => Role::Frequency,
            "pitch" => Role::Pitch,
            _ => Role::Reference,
        }
    }
}

/// A loaded dictionary and what it is for.
#[derive(Debug, Clone)]
pub struct Dictionary {
    pub id: i64,
    pub title: String,
    pub source_path: String,
    pub role: Role,
    /// Who answers first among equals: the popup's page order, and which
    /// frequency list is the reader's. Defaults to the id, so install order is
    /// the order until someone says otherwise.
    pub priority: i64,
}

/// Drop a cached dictionary and everything it holds.
///
/// The three payload tables key on `dictionary_id` and nothing cascades, so a
/// row deleted from `dictionaries` alone would leave its entries behind —
/// invisible to `list`, still answering the wordhood gate. All four go in one
/// transaction.
///
/// Callers refresh `vocabulary`'s dictionary flags afterwards: which
/// dictionaries hold a term is cached on the row, and dropping one makes that
/// cache wrong until it is recomputed.
pub async fn remove_dictionary(pool: &SqlitePool, id: i64) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let mut removed = 0;
    for table in [
        "dictionary_entries",
        "dictionary_pitch",
        "dictionary_frequency",
    ] {
        removed += sqlx::query(&format!("DELETE FROM {table} WHERE dictionary_id = ?"))
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    }
    sqlx::query("DELETE FROM dictionaries WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(removed)
}

pub async fn list_dictionaries(pool: &SqlitePool) -> Result<Vec<Dictionary>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, title, source_path, role, priority FROM dictionaries \
         ORDER BY priority, id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| Dictionary {
            id: r.get("id"),
            title: r.get("title"),
            source_path: r.get("source_path"),
            role: Role::parse(r.get("role")),
            priority: r.get("priority"),
        })
        .collect())
}

/// Repoint a cached dictionary at the zip it now lives in.
///
/// `source_path` is the cache key, so moving a zip would otherwise re-import it
/// from scratch and leave the old row behind as a duplicate.
/// `backfill_sequences` also resolves the zip through this column, and skips a
/// dictionary whose path no longer exists.
pub async fn set_source_path(
    pool: &SqlitePool,
    id: i64,
    source_path: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE dictionaries SET source_path = ? WHERE id = ?")
        .bind(source_path)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_priority(pool: &SqlitePool, id: i64, priority: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE dictionaries SET priority = ? WHERE id = ?")
        .bind(priority)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_role(pool: &SqlitePool, id: i64, role: Role) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE dictionaries SET role = ? WHERE id = ?")
        .bind(role.as_str())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// The master dictionary, if one has been marked.
pub async fn master(pool: &SqlitePool) -> Result<Option<Dictionary>, sqlx::Error> {
    Ok(list_dictionaries(pool)
        .await?
        .into_iter()
        .find(|d| d.role == Role::Master))
}

/// The frequency list every *reader-facing* rank comes from — the underline in
/// kotodex-server's feed, both halves of its frequency triage, the popup, and the
/// rank on a mined card. Whichever dictionary holds [`Role::Frequency`], lowest
/// id first where several do.
///
/// **Not loaded is not an error.** No frequency dictionary means no underline,
/// no rank pill and no by-frequency ordering — never a failure.
///
/// **Two frequency dictionaries answer different questions.** BCCWJ arbitrates
/// *between spellings of one reading* inside the tokenizer ([`frequency_ranks`],
/// [`preferred_readings`]), which needs a rank per `(spelling, reading)` and
/// asks for BCCWJ by name. These features ask "how common is this word, at
/// all", and BCCWJ is the wrong list for it twice over:
///
/// - It files a word only under the corpus's orthography, with no kana row —
///   いただく is present as 頂く/いただく alone, so a line that writes it in kana
///   gets no rank and no underline.
/// - Its corpus is newspapers and government prose, and "common in Japanese"
///   has to mean common in the fiction being read, or the underline marks the
///   wrong gaps.
///
/// It lives here rather than in whichever tool asked first because it is one
/// claim about which words are common, made in several places across two
/// binaries.
pub async fn reader_frequency(pool: &SqlitePool) -> Result<Option<Dictionary>, sqlx::Error> {
    Ok(list_dictionaries(pool)
        .await?
        .into_iter()
        .find(|d| d.role == Role::Frequency))
}

/// What the installer assigns [`Role::Frequency`] to when it downloads the
/// free dictionaries. A default for setup, never a runtime lookup — ask
/// [`reader_frequency`] instead.
pub const READER_FREQUENCY: &str = "Jiten";

/// A loaded dictionary by exact title, for a source with no `role` of its own
/// to select on — the BCCWJ frequency table is loaded as a plain `reference`
/// dictionary, so its id has to be found by name.
pub async fn by_title(pool: &SqlitePool, title: &str) -> Result<Option<Dictionary>, sqlx::Error> {
    Ok(list_dictionaries(pool)
        .await?
        .into_iter()
        .find(|d| d.title == title))
}

/// Distinct readings the master dictionary lists for a headword.
///
/// The shared last step of resolving a reading-less external source (an Anki
/// card, a frequency-list term) into a ledger key: zero readings means the
/// term isn't master vocabulary, one is unambiguous, and more than one is a
/// homograph the caller has to decide what to do with rather than guess.
pub async fn master_readings(pool: &SqlitePool, term: &str) -> Result<Vec<String>, sqlx::Error> {
    let Some(master) = master(pool).await? else {
        return Ok(Vec::new());
    };
    let entries = lookup_dictionary_entries(pool, master.id, term).await?;
    let mut readings: Vec<String> = entries.into_iter().map(|e| e.reading).collect();
    readings.sort();
    readings.dedup();
    Ok(readings)
}

/// Every reading any loaded dictionary gives this headword.
///
/// The fallback for a term the master does not list. The Anki import stores an
/// empty reading when [`master_readings`] comes back empty, which is right for a
/// kana headword and wrong otherwise — 冪等性 is a real word with a real reading
/// that Sankoku does not carry, and an empty key either strands its judgement or
/// duplicates a row the tokenizer already made.
///
/// Deliberately wider than the master gate: this answers "how is it read", which
/// any dictionary may answer. It admits nothing into the vocabulary scale.
pub async fn any_readings(pool: &SqlitePool, term: &str) -> Result<Vec<String>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT reading FROM dictionary_entries \
         WHERE term = ? AND reading != '' AND reading != term",
    )
    .bind(term)
    .fetch_all(pool)
    .await?;
    let mut readings: Vec<String> = rows.into_iter().map(|(r,)| r).collect();
    readings.sort();
    readings.dedup();
    Ok(readings)
}

/// Which dictionary to call master when the configured one is not installed.
///
/// A stranger's set has no 三省堂 in it, and a master is what segmentation and
/// the vocabulary scale are measured against — so picking a defensible one
/// beats having none. The standard monolinguals first, then the *smallest*
/// dictionary: a monolingual is an order of magnitude smaller than a bilingual
/// like Jitendex, whose headwords include every compositional compound and
/// orthographic variant and would make "I know N words" meaningless.
///
/// Returns an index into `all`. The count query walks `dictionary_entries`, so
/// it runs only on the no-master path — once, at startup.
async fn fallback_master(
    pool: &SqlitePool,
    all: &[Dictionary],
) -> Result<Option<usize>, sqlx::Error> {
    if let Some(i) = all.iter().position(|d| d.role == Role::Standard) {
        return Ok(Some(i));
    }
    let counts: Vec<(i64, i64)> =
        sqlx::query_as("SELECT dictionary_id, COUNT(*) FROM dictionary_entries GROUP BY 1")
            .fetch_all(pool)
            .await?;
    if let Some(i) = counts
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .min_by_key(|(_, n)| *n)
        .and_then(|(id, _)| all.iter().position(|d| d.id == id))
    {
        return Ok(Some(i));
    }
    // Nothing has been counted yet — the entries land after the row does. Take
    // the first dictionary that could hold definitions at all.
    Ok(all
        .iter()
        .position(|d| !matches!(d.role, Role::Frequency | Role::Pitch)))
}

/// Mark the dictionary whose `source_path` ends with `marker` as master, and
/// demote any other master. Idempotent, and a no-op when nothing matches — the
/// master is named in configuration, and a config naming a dictionary that
/// isn't loaded must not clear the one that is.
///
/// Called at startup rather than at import time, so changing the setting takes
/// effect on the next run without re-importing anything.
pub async fn ensure_master(pool: &SqlitePool, marker: &str) -> Result<Option<i64>, sqlx::Error> {
    let all = list_dictionaries(pool).await?;
    let named = (!marker.is_empty())
        .then(|| {
            all.iter()
                .position(|d| d.source_path.ends_with(marker) || d.title == marker)
        })
        .flatten();
    let index = match named {
        Some(i) => i,
        None => {
            // A config naming a dictionary that isn't loaded must not clear the
            // master that is; the fallback is only for having none at all.
            if all.iter().any(|d| d.role == Role::Master) {
                return Ok(None);
            }
            match fallback_master(pool, &all).await? {
                Some(i) => i,
                None => return Ok(None),
            }
        }
    };
    let target = &all[index];
    for d in &all {
        if d.id == target.id && d.role != Role::Master {
            set_role(pool, d.id, Role::Master).await?;
        } else if d.id != target.id && d.role == Role::Master {
            set_role(pool, d.id, Role::Reference).await?;
        }
    }
    Ok(Some(target.id))
}

/// Default master: Sankoku (三省堂国語辞典), the only loaded dictionary whose
/// headword count is a vocabulary scale rather than a phrase index.
pub const DEFAULT_MASTER: &str = "三省堂国語辞典第八版.zip";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::Knowledge;

    async fn with_dicts(rows: &[(&str, &str)]) -> Knowledge {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("jp-core-dicts-{nanos}.db"));
        let k = Knowledge::open(path.to_str().unwrap()).await.unwrap();
        for (title, source) in rows {
            sqlx::query("INSERT INTO dictionaries (title, source_path) VALUES (?, ?)")
                .bind(title)
                .bind(source)
                .execute(k.pool())
                .await
                .unwrap();
        }
        k
    }

    #[tokio::test]
    async fn a_new_import_is_reference_until_told_otherwise() {
        let k = with_dicts(&[("Jitendex", "/x/jitendex-yomitan.zip")]).await;
        let all = list_dictionaries(k.pool()).await.unwrap();
        assert_eq!(all[0].role, Role::Reference);
        assert!(master(k.pool()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn ensure_master_marks_one_and_demotes_the_rest() {
        let k = with_dicts(&[
            ("Jitendex", "/x/jitendex-yomitan.zip"),
            ("三省堂国語辞典 第八版", "/x/三省堂国語辞典第八版.zip"),
            ("NHK", "/x/NHK2016.zip"),
        ])
        .await;

        // Matched on the tail of the path, so the configured value can be a
        // bare filename while the row holds an absolute path.
        let id = ensure_master(k.pool(), DEFAULT_MASTER).await.unwrap();
        assert_eq!(id, Some(2));
        assert_eq!(
            master(k.pool()).await.unwrap().unwrap().title,
            "三省堂国語辞典 第八版"
        );

        // Idempotent, and switching masters demotes the old one.
        ensure_master(k.pool(), DEFAULT_MASTER).await.unwrap();
        assert_eq!(master(k.pool()).await.unwrap().unwrap().id, 2);
        ensure_master(k.pool(), "jitendex-yomitan.zip")
            .await
            .unwrap();
        let all = list_dictionaries(k.pool()).await.unwrap();
        assert_eq!(all[0].role, Role::Master);
        assert_eq!(all[1].role, Role::Reference, "the old master steps down");
    }

    #[tokio::test]
    async fn a_set_without_the_configured_master_still_gets_one() {
        // A stranger installs Jitendex and nothing else. Segmentation and the
        // vocabulary scale both need a master, so the smallest dictionary
        // present takes the role rather than the set having none.
        let k = with_dicts(&[("Jitendex", "/x/jitendex-yomitan.zip")]).await;
        assert!(
            ensure_master(k.pool(), DEFAULT_MASTER)
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(master(k.pool()).await.unwrap().unwrap().title, "Jitendex");
    }

    #[tokio::test]
    async fn a_standard_monolingual_outranks_a_bigger_dictionary_as_fallback() {
        let k = with_dicts(&[
            ("Jitendex", "/x/jitendex-yomitan.zip"),
            ("明鏡国語辞典 第三版", "/x/明鏡国語辞典第三版.zip"),
        ])
        .await;
        set_role(k.pool(), 2, Role::Standard).await.unwrap();
        ensure_master(k.pool(), DEFAULT_MASTER).await.unwrap();
        assert_eq!(
            master(k.pool()).await.unwrap().unwrap().title,
            "明鏡国語辞典 第三版"
        );
    }

    #[tokio::test]
    async fn a_master_that_is_not_loaded_leaves_the_current_one_alone() {
        // Otherwise a stale config entry would silently clear the vocabulary
        // denominator, and "I know N words" would quietly become zero.
        let k = with_dicts(&[("三省堂国語辞典 第八版", "/x/三省堂国語辞典第八版.zip")]).await;
        ensure_master(k.pool(), DEFAULT_MASTER).await.unwrap();
        assert!(
            ensure_master(k.pool(), "not-installed.zip")
                .await
                .unwrap()
                .is_none()
        );
        assert!(master(k.pool()).await.unwrap().is_some(), "still marked");
    }

    /// The two dictionaries spell a kana headword's reading differently — one
    /// repeats the headword, the other leaves it empty — and an exact
    /// `reading = reading` join silently drops every one of them.
    #[tokio::test]
    async fn a_kana_headword_joins_across_the_two_reading_conventions() {
        let k = with_dicts(&[
            ("Sankoku", "/x/sankoku.zip"),
            ("Jitendex", "/x/jitendex.zip"),
        ])
        .await;
        set_role(k.pool(), 1, Role::Master).await.unwrap();

        // Sankoku stores the kana headword with no reading at all.
        for (dict, term, reading, seq) in [
            (1i64, "あそこ", "", None),
            (1, "零れ落ちる", "こぼれおちる", None),
            // Jitendex repeats the headword as the reading, and carries ids.
            (2, "あそこ", "あそこ", Some(1000320i64)),
            (2, "かしこ", "かしこ", Some(1000320)),
            (2, "零れ落ちる", "こぼれおちる", Some(2002270)),
            (2, "こぼれ落ちる", "こぼれおちる", Some(2002270)),
        ] {
            sqlx::query(
                "INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json, sequence) \
                 VALUES (?, ?, ?, '[]', ?)",
            )
            .bind(dict).bind(term).bind(reading).bind(seq)
            .execute(k.pool()).await.unwrap();
        }

        let map = master_forms_by_sequence(k.pool()).await.unwrap();

        let mut asoko = map.get(&1000320).cloned().unwrap_or_default();
        asoko.sort();
        assert_eq!(
            asoko,
            vec![("あそこ".to_string(), "あそこ".to_string())],
            "あそこ joins despite the empty reading; かしこ is not in the master dictionary and must not ride in on the shared entry id"
        );

        let mut koboreru = map.get(&2002270).cloned().unwrap_or_default();
        koboreru.sort();
        assert_eq!(
            koboreru,
            vec![("零れ落ちる".to_string(), "こぼれおちる".to_string())],
            "only the spelling the master dictionary lists"
        );
    }

    /// An entry keeps every way a word has ever been written and read, and tags
    /// the dead ones with a negative score. A card for 三人 claims さんにん; it
    /// does not claim みたり, which the reader has very likely never met.
    #[tokio::test]
    async fn a_form_the_entry_tags_as_dead_is_not_one_it_claims() {
        let k = with_dicts(&[
            ("Sankoku", "/x/sankoku.zip"),
            ("Jitendex", "/x/jitendex.zip"),
        ])
        .await;
        set_role(k.pool(), 1, Role::Master).await.unwrap();

        // Sankoku lists both readings; JMdict scores みたり -102.
        for (dict, term, reading, seq, score) in [
            (1i64, "三人", "さんにん", None, 0i64),
            (1, "三人", "みたり", None, 0),
            (2, "三人", "さんにん", Some(1301000i64), 200),
            (2, "三人", "みたり", Some(1301000), -102),
        ] {
            sqlx::query(
                "INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json, sequence, score) \
                 VALUES (?, ?, ?, '[]', ?, ?)",
            )
            .bind(dict).bind(term).bind(reading).bind(seq).bind(score)
            .execute(k.pool()).await.unwrap();
        }

        let map = master_forms_by_sequence(k.pool()).await.unwrap();
        assert_eq!(
            map.get(&1301000).cloned().unwrap_or_default(),
            vec![("三人".to_string(), "さんにん".to_string())],
            "みたり is tagged dead and must not ride in on the shared entry id"
        );
    }

    /// The live spellings still fan out — that is what stops triage asking
    /// about 回る and 廻る separately.
    #[tokio::test]
    async fn the_spellings_an_entry_still_uses_all_come_through() {
        let k = with_dicts(&[
            ("Sankoku", "/x/sankoku.zip"),
            ("Jitendex", "/x/jitendex.zip"),
        ])
        .await;
        set_role(k.pool(), 1, Role::Master).await.unwrap();

        for (dict, term, seq, score) in [
            (1i64, "回る", None, 0i64),
            (1, "廻る", None, 0),
            (2, "回る", Some(1604300i64), 200),
            (2, "廻る", Some(1604300), 199),
        ] {
            sqlx::query(
                "INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json, sequence, score) \
                 VALUES (?, ?, 'まわる', '[]', ?, ?)",
            )
            .bind(dict).bind(term).bind(seq).bind(score)
            .execute(k.pool()).await.unwrap();
        }

        let mut forms = master_forms_by_sequence(k.pool())
            .await
            .unwrap()
            .get(&1604300)
            .cloned()
            .unwrap_or_default();
        forms.sort();
        assert_eq!(forms.len(), 2, "both live spellings: {forms:?}");
    }

    /// An entry groups every way its word is *said*, and those are separate
    /// claims. 出入り is でいり at 200 and ではいり at 99 — one entry, one card,
    /// and knowing the word says nothing about the second reading.
    #[tokio::test]
    async fn a_secondary_reading_is_a_claim_the_card_does_not_make() {
        let k = with_dicts(&[
            ("Sankoku", "/x/sankoku.zip"),
            ("Jitendex", "/x/jitendex.zip"),
        ])
        .await;
        set_role(k.pool(), 1, Role::Master).await.unwrap();

        for (dict, term, reading, seq, score) in [
            (1i64, "出入り", "でいり", None, 0i64),
            (1, "出入り", "ではいり", None, 0),
            (2, "出入り", "でいり", Some(1339910i64), 200),
            (2, "出入り", "ではいり", Some(1339910), 99),
        ] {
            sqlx::query(
                "INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json, sequence, score) \
                 VALUES (?, ?, ?, '[]', ?, ?)",
            )
            .bind(dict).bind(term).bind(reading).bind(seq).bind(score)
            .execute(k.pool()).await.unwrap();
        }

        let map = master_forms_by_sequence(k.pool()).await.unwrap();
        assert_eq!(
            map.get(&1339910).cloned().unwrap_or_default(),
            vec![("出入り".to_string(), "でいり".to_string())],
            "ではいり is a reading of its own, not a spelling of でいり"
        );
    }

    /// But a variant *spelling* of the primary reading still comes through,
    /// even a tier down: 次々 and 次次 are both つぎつぎ and collapse onto one
    /// lexeme, so admitting the second costs nothing and saves a triage
    /// question.
    #[tokio::test]
    async fn a_variant_spelling_of_the_living_reading_still_comes_through() {
        let k = with_dicts(&[
            ("Sankoku", "/x/sankoku.zip"),
            ("Jitendex", "/x/jitendex.zip"),
        ])
        .await;
        set_role(k.pool(), 1, Role::Master).await.unwrap();

        for (dict, term, seq, score) in [
            (1i64, "次々", None, 0i64),
            (1, "次次", None, 0),
            (2, "次々", Some(1597850i64), 200),
            (2, "次次", Some(1597850), 99),
        ] {
            sqlx::query(
                "INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json, sequence, score) \
                 VALUES (?, ?, 'つぎつぎ', '[]', ?, ?)",
            )
            .bind(dict).bind(term).bind(seq).bind(score)
            .execute(k.pool()).await.unwrap();
        }

        let forms = master_forms_by_sequence(k.pool())
            .await
            .unwrap()
            .get(&1597850)
            .cloned()
            .unwrap_or_default();
        assert_eq!(forms.len(), 2, "both spellings of つぎつぎ: {forms:?}");
    }

    #[tokio::test]
    async fn an_entry_with_no_master_spelling_resolves_to_nothing() {
        let k = with_dicts(&[
            ("Sankoku", "/x/sankoku.zip"),
            ("Jitendex", "/x/jitendex.zip"),
        ])
        .await;
        set_role(k.pool(), 1, Role::Master).await.unwrap();
        // A surname: Jitendex has it, Sankoku does not.
        sqlx::query(
            "INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json, sequence) \
             VALUES (2, '長谷川', 'はせがわ', '[]', 2082430)",
        )
        .execute(k.pool()).await.unwrap();

        let map = master_forms_by_sequence(k.pool()).await.unwrap();
        assert!(!map.contains_key(&2082430), "a name imports as nothing");
    }

    /// Sets up Sankoku (master, id 1), Jitendex (popularity, id 2) and BCCWJ
    /// (corpus, id 3) with one term's worth of rows.
    async fn with_readings(
        master: &[(&str, &str)],
        popularity: &[(&str, &str, i64)],
        corpus: &[(&str, &str, i64)],
    ) -> Knowledge {
        let k = with_dicts(&[
            ("Sankoku", "/x/sankoku.zip"),
            ("Jitendex", "/x/jitendex.zip"),
            ("BCCWJ", "/x/bccwj.zip"),
        ])
        .await;
        set_role(k.pool(), 1, Role::Master).await.unwrap();
        for (term, reading) in master {
            sqlx::query(
                "INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json) \
                 VALUES (1, ?, ?, '[]')",
            )
            .bind(term)
            .bind(reading)
            .execute(k.pool())
            .await
            .unwrap();
        }
        for (term, reading, score) in popularity {
            sqlx::query(
                "INSERT INTO dictionary_entries (dictionary_id, term, reading, score, definitions_json) \
                 VALUES (2, ?, ?, ?, '[]')",
            )
            .bind(term)
            .bind(reading)
            .bind(score)
            .execute(k.pool())
            .await
            .unwrap();
        }
        for (term, reading, rank) in corpus {
            sqlx::query(
                "INSERT INTO dictionary_frequency (dictionary_id, term, reading, frequency) \
                 VALUES (3, ?, ?, ?)",
            )
            .bind(term)
            .bind(reading)
            .bind(rank)
            .execute(k.pool())
            .await
            .unwrap();
        }
        k
    }

    /// The case this exists for. Sudachi reads a bare 私 as わたくし and the
    /// corpus agrees with it, being annotated with the same UniDic — so only the
    /// popularity tags can say otherwise, and the corpus is left to break the
    /// tie between the two readings that are current.
    #[tokio::test]
    async fn a_dead_reading_is_corrected_to_the_living_one() {
        let k = with_readings(
            &[("私", "わたし"), ("私", "わたくし"), ("私", "あたし")],
            &[
                ("私", "わたし", 200),
                ("私", "あたし", 200),
                ("私", "わたくし", 0),
            ],
            &[
                ("私", "わたし", 182),
                ("私", "あたし", 678),
                ("私", "わたくし", 47),
            ],
        )
        .await;

        let prefs = preferred_readings(k.pool(), 1, 2, 3).await.unwrap();
        let p = prefs.get("私").expect("私 needs a preferred reading");
        assert_eq!(
            p.preferred, "わたし",
            "the commonest of the current readings"
        );
        assert!(
            p.acceptable.contains("あたし"),
            "also current, so not corrected"
        );
        assert!(!p.acceptable.contains("わたくし"));
    }

    /// Two readings the language uses side by side. The sentence decides, and
    /// nothing here may overrule it.
    #[tokio::test]
    async fn readings_that_are_both_current_get_no_preference() {
        let k = with_readings(
            &[("何", "なに"), ("何", "なん")],
            &[("何", "なに", 200), ("何", "なん", 200)],
            &[("何", "なに", 30), ("何", "なん", 40)],
        )
        .await;
        assert!(
            preferred_readings(k.pool(), 1, 2, 3)
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// A negative score is JMdict saying the *spelling* is unusual — 居る is
    /// usually written いる in kana — not that the reading is dead. Believing it
    /// would rewrite every kanji 居る to おる.
    #[tokio::test]
    async fn a_reading_tagged_odd_is_left_alone_rather_than_corrected() {
        let k = with_readings(
            &[("居る", "いる"), ("居る", "おる")],
            &[("居る", "いる", -101), ("居る", "おる", 200)],
            &[("居る", "いる", 13), ("居る", "おる", 107)],
        )
        .await;
        let prefs = preferred_readings(k.pool(), 1, 2, 3).await.unwrap();
        assert!(
            prefs
                .get("居る")
                .is_none_or(|p| p.acceptable.contains("いる")),
            "いる must never be corrected away"
        );
    }

    /// Nothing is tagged common, so there is no reading to correct *to*: 何時 is
    /// なんどき at 0 and いつ at -101, and preferring なんどき inverts the truth.
    #[tokio::test]
    async fn a_headword_with_no_common_reading_gets_no_preference() {
        let k = with_readings(
            &[("何時", "いつ"), ("何時", "なんどき")],
            &[("何時", "いつ", -101), ("何時", "なんどき", 0)],
            &[("何時", "いつ", 200), ("何時", "なんどき", 9000)],
        )
        .await;
        assert!(
            preferred_readings(k.pool(), 1, 2, 3)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
