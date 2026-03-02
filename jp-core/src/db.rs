use std::collections::HashSet;

use sqlx::{Row, SqlitePool};

use crate::dictionary::{DictionaryEntry, PitchEntry};

const MIGRATION_DICT: &str = include_str!("../migrations/002_create_dictionary_tables.sql");
const MIGRATION_PITCH: &str = include_str!("../migrations/003_create_pitch_tables.sql");

/// Run dictionary-related migrations (idempotent).
/// Call this during application startup after creating the pool.
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(MIGRATION_DICT).execute(pool).await?;
    sqlx::raw_sql(MIGRATION_PITCH).execute(pool).await?;

    // Replace old single-column indexes with composite ones
    sqlx::raw_sql(
        "DROP INDEX IF EXISTS idx_dictionary_entries_term;\
         DROP INDEX IF EXISTS idx_dictionary_entries_dict;\
         DROP INDEX IF EXISTS idx_dictionary_pitch_term;\
         DROP INDEX IF EXISTS idx_dictionary_pitch_dict;",
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Load all distinct headwords from dictionary_entries.
/// Used at startup to build the set for dictionary-aware tokenization.
pub async fn get_all_headwords(pool: &SqlitePool) -> Result<HashSet<String>, sqlx::Error> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT DISTINCT term FROM dictionary_entries")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|(term,)| term).collect())
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

    let row = sqlx::query(
        "INSERT INTO dictionaries (title, source_path) VALUES (?, ?) RETURNING id",
    )
    .bind(title)
    .bind(source_path)
    .fetch_one(&mut *tx)
    .await?;
    let dict_id: i64 = row.get("id");

    for entry in entries {
        let definitions_json = serde_json::to_string(&entry.definitions)
            .unwrap_or_else(|_| "[]".into());
        sqlx::query(
            "INSERT INTO dictionary_entries (dictionary_id, term, reading, score, definitions_json) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(dict_id)
        .bind(&entry.term)
        .bind(&entry.reading)
        .bind(entry.score)
        .bind(&definitions_json)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(dict_id)
}

pub async fn lookup_dictionary_entries(
    pool: &SqlitePool,
    dictionary_id: i64,
    term: &str,
) -> Result<Vec<DictionaryEntry>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT term, reading, score, definitions_json FROM dictionary_entries WHERE dictionary_id = ? AND term = ?",
    )
    .bind(dictionary_id)
    .bind(term)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let json_str: String = r.get("definitions_json");
            let definitions: Vec<String> =
                serde_json::from_str(&json_str).unwrap_or_default();
            DictionaryEntry {
                term: r.get("term"),
                reading: r.get("reading"),
                score: r.get("score"),
                definitions,
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

/// Check whether any pitch entries exist for a dictionary.
pub async fn has_pitch_entries(
    pool: &SqlitePool,
    dictionary_id: i64,
) -> Result<bool, sqlx::Error> {
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM dictionary_pitch WHERE dictionary_id = ?",
    )
    .bind(dictionary_id)
    .fetch_one(pool)
    .await?;
    Ok(count.0 > 0)
}
