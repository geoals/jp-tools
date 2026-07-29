//! The dictionary cache: terms, readings, pitch accents and frequency ranks,
//! imported from Yomitan zips and queried by every tool that needs to know
//! whether a string is a word.
//!
//! ## Roles, and why a term's dictionary matters
//!
//! Three dictionaries are loaded today — Sankoku (三省堂国語辞典, ~82k terms),
//! Jitendex (~408k) and NHK (pitch only) — and they cannot be pooled into one
//! "is it a word" set, because they disagree about what a word *is*. 335,540
//! Jitendex terms are absent from Sankoku: phrasal expressions (`ああ見えても`),
//! compositional compounds (`あいうえお順`), and every orthographic variant of a
//! technical term each get their own headword. A monolingual dictionary lists
//! such phrases *under* a headword; Jitendex makes them headwords. So a vocab
//! size counted against Jitendex means nothing.
//!
//! Hence [`Role`], and two different thresholds from the same data:
//!
//! - **wordhood gate** (is this token worth surfacing at all?) — lenient: any
//!   dictionary will do.
//! - **vocabulary denominator** ("I know N words") — strict: the master only.
//!   Sankoku's ~82k ceiling is a real vocabulary scale.
//!
//! Adding a dictionary must therefore never move the denominator, which is what
//! the role is for: a new import is `reference` until someone says otherwise.

use std::collections::{HashMap, HashSet};

use sqlx::{Row, SqlitePool};

use crate::dictionary::{DictionaryEntry, PitchEntry};

/// Load all distinct headwords from dictionary_entries.
/// Used at startup to build the set for dictionary-aware tokenization.
pub async fn get_all_headwords(pool: &SqlitePool) -> Result<HashSet<String>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT DISTINCT term FROM dictionary_entries")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(term,)| term).collect())
}

/// The master dictionary's headwords, for decomposing compounds it does not
/// list into parts it does.
pub async fn master_headwords(pool: &SqlitePool) -> Result<HashSet<String>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT de.term FROM dictionary_entries de \
         JOIN dictionaries d ON d.id = de.dictionary_id WHERE d.role = 'master'",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(term,)| term).collect())
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

/// Load all distinct terms and readings from dictionary_entries.
/// Broader than `get_all_headwords` — includes kana readings so that
/// hiragana-only lemmas like いう match dictionary entry 言う (reading いう).
pub async fn get_all_dictionary_forms(pool: &SqlitePool) -> Result<HashSet<String>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT term FROM dictionary_entries UNION SELECT DISTINCT reading FROM dictionary_entries",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(form,)| form).collect())
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

/// Insert a dictionary and all its entries in a single transaction.
/// Returns the dictionary id. If interrupted, the transaction rolls back
/// so no partial data is left behind.
pub async fn import_dictionary(
    pool: &SqlitePool,
    title: &str,
    source_path: &str,
    entries: &[DictionaryEntry],
) -> Result<i64, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let row =
        sqlx::query("INSERT INTO dictionaries (title, source_path) VALUES (?, ?) RETURNING id")
            .bind(title)
            .bind(source_path)
            .fetch_one(&mut *tx)
            .await?;
    let dict_id: i64 = row.get("id");

    for entry in entries {
        let definitions_json =
            serde_json::to_string(&entry.definitions).unwrap_or_else(|_| "[]".into());
        sqlx::query(
            "INSERT INTO dictionary_entries (dictionary_id, term, reading, score, definitions_json, sequence) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(dict_id)
        .bind(&entry.term)
        .bind(&entry.reading)
        .bind(entry.score)
        .bind(&definitions_json)
        .bind(entry.sequence)
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

/// Attach entry ids to a dictionary cached before `sequence` existed.
///
/// Matched on `(term, reading)` — the same pair the entries were stored
/// under, so this is a straight update and never invents a row. Marks the
/// dictionary checked either way.
pub async fn backfill_sequences(
    pool: &SqlitePool,
    dictionary_id: i64,
    entries: &[DictionaryEntry],
) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let mut updated = 0;
    for entry in entries {
        let Some(seq) = entry.sequence else { continue };
        updated += sqlx::query(
            "UPDATE dictionary_entries SET sequence = ? \
             WHERE dictionary_id = ? AND term = ? AND reading = ?",
        )
        .bind(seq)
        .bind(dictionary_id)
        .bind(&entry.term)
        .bind(&entry.reading)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    sqlx::query("UPDATE dictionaries SET seq_checked = 1 WHERE id = ?")
        .bind(dictionary_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(updated)
}

/// The dictionary whose entry ids define lexemes — the one carrying the most
/// of them.
///
/// Deliberately not the master dictionary. Sankoku is monolingual and
/// publishes no stable ids, so the question "are these two spellings one
/// word?" can only be answered by a reference dictionary, even though the
/// master alone decides what counts as vocabulary. The two roles are
/// independent and both are needed.
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
/// The join is what makes an id-keyed import safe. jiten.moe exports JMdict
/// entry ids and nothing else: an id names a *word*, but a word is spelt
/// several ways and the ledger keys on spellings. This resolves one to the
/// other, and the master join is the vocabulary gate — 長谷川 and JMdict's
/// multi-word phrases resolve to no master form and so import as nothing.
///
/// Readings are compared under the kana convention both dictionaries use:
/// an entry whose headword is already kana stores the reading either as the
/// headword again or not at all, so both sides are normalized before matching.
/// Without that, every kana headword failed to join and the import silently
/// dropped a third of its rows.
///
/// Returned whole rather than queried per id: an import asks about ~13k ids,
/// and one scan of the join beats 13k round trips.
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
        "SELECT DISTINCT j.sequence AS seq, j.term AS term, \
                COALESCE(NULLIF(j.reading, ''), j.term) AS reading \
         FROM dictionary_entries j \
         JOIN dictionary_entries m \
           ON m.dictionary_id = ? AND m.term = j.term \
          AND COALESCE(NULLIF(m.reading, ''), m.term) \
              = COALESCE(NULLIF(j.reading, ''), j.term) \
         WHERE j.dictionary_id = ? AND j.sequence IS NOT NULL",
    )
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
        "SELECT term, reading, score, definitions_json, sequence FROM dictionary_entries WHERE dictionary_id = ? AND term = ?",
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
            }
        })
        .collect())
}

/// Insert pitch accent entries for a dictionary within a transaction.
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

/// Insert frequency entries for a dictionary within a transaction.
/// Batched multi-row inserts: frequency dictionaries can hold 1M+ entries
/// (e.g. BCCWJ), and per-row round-trips make import take minutes.
pub async fn insert_frequency_entries(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    dictionary_id: i64,
    entries: &[(String, i64)],
) -> Result<(), sqlx::Error> {
    // 300 rows × 3 binds = 900 variables, safely under SQLite's limit
    for chunk in entries.chunks(300) {
        let placeholders = vec!["(?, ?, ?)"; chunk.len()].join(",");
        let sql = format!(
            "INSERT INTO dictionary_frequency (dictionary_id, term, frequency) VALUES {placeholders}"
        );
        let mut query = sqlx::query(&sql);
        for (term, freq) in chunk {
            query = query.bind(dictionary_id).bind(term).bind(freq);
        }
        query.execute(&mut **tx).await?;
    }
    Ok(())
}

/// Look up the best (lowest) frequency rank for a term.
/// Frequency dicts often carry multiple entries per term (readings,
/// short/long unit words); the minimum rank is the most common usage.
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

/// Check whether any frequency entries exist for a dictionary.
pub async fn has_frequency_entries(
    pool: &SqlitePool,
    dictionary_id: i64,
) -> Result<bool, sqlx::Error> {
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM dictionary_frequency WHERE dictionary_id = ?")
            .bind(dictionary_id)
            .fetch_one(pool)
            .await?;
    Ok(count.0 > 0)
}

/// Check whether any pitch entries exist for a dictionary.
pub async fn has_pitch_entries(pool: &SqlitePool, dictionary_id: i64) -> Result<bool, sqlx::Error> {
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM dictionary_pitch WHERE dictionary_id = ?")
            .bind(dictionary_id)
            .fetch_one(pool)
            .await?;
    Ok(count.0 > 0)
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
    /// A name dictionary: a term in here but not in the master is a name, not
    /// vocabulary. (None loaded yet; the schema is ready for one.)
    Name,
    /// Everything else — bilingual dictionaries, pitch, frequency. Counts for
    /// the wordhood gate, never toward the vocabulary total.
    Reference,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Master => "master",
            Role::Name => "name",
            Role::Reference => "reference",
        }
    }

    pub fn parse(s: &str) -> Role {
        match s {
            "master" => Role::Master,
            "name" => Role::Name,
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
}

pub async fn list_dictionaries(pool: &SqlitePool) -> Result<Vec<Dictionary>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, title, source_path, role FROM dictionaries ORDER BY id")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| Dictionary {
            id: r.get("id"),
            title: r.get("title"),
            source_path: r.get("source_path"),
            role: Role::parse(r.get("role")),
        })
        .collect())
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
/// The fallback for a term the master does not list. Pass 1 stores an empty
/// reading when [`master_readings`] comes back empty, which is right for a
/// kana headword (the ledger's key convention) and wrong for everything else:
/// 復号 and 冪等性 are real words with real readings that Sankoku simply does
/// not carry, and keying them on an empty reading either strands the judgement
/// on a row nothing writes to or duplicates a row the tokenizer already made.
///
/// Deliberately wider than the master gate. This answers "how is it read",
/// which any dictionary may answer; it does not admit anything into the
/// vocabulary scale, which only the master and [`super::vocabulary::COUNTS_AS_VOCAB`] decide.
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

/// Mark the dictionary whose `source_path` ends with `marker` as master, and
/// demote any other master. Idempotent, and a no-op when nothing matches — the
/// master is named in configuration, and a config naming a dictionary that
/// isn't loaded must not clear the one that is.
///
/// Called at startup rather than at import time so that changing the setting
/// takes effect on the next run, without re-importing 400k entries.
pub async fn ensure_master(pool: &SqlitePool, marker: &str) -> Result<Option<i64>, sqlx::Error> {
    if marker.is_empty() {
        return Ok(None);
    }
    let all = list_dictionaries(pool).await?;
    let Some(target) = all
        .iter()
        .find(|d| d.source_path.ends_with(marker) || d.title == marker)
    else {
        return Ok(None);
    };
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
        let k = with_dicts(&[("Sankoku", "/x/sankoku.zip"), ("Jitendex", "/x/jitendex.zip")]).await;
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

    #[tokio::test]
    async fn an_entry_with_no_master_spelling_resolves_to_nothing() {
        let k = with_dicts(&[("Sankoku", "/x/sankoku.zip"), ("Jitendex", "/x/jitendex.zip")]).await;
        set_role(k.pool(), 1, Role::Master).await.unwrap();
        // A surname: Jitendex has it, Sankoku does not.
        sqlx::query(
            "INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json, sequence) \
             VALUES (2, '長谷川', 'はせがわ', '[]', 2082430)",
        )
        .execute(k.pool()).await.unwrap();

        let map = master_forms_by_sequence(k.pool()).await.unwrap();
        assert!(map.get(&2082430).is_none(), "a name imports as nothing");
    }
}
