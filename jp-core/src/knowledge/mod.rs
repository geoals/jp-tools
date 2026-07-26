//! `knowledge.db` — the shared database, and the handle that opens it.
//!
//! One file, owned by jp-core, holding everything that is *about the language
//! and what has been read* rather than about one app's workflow:
//!
//! - the dictionary cache ([`dictionaries`]) — terms, readings, pitch, frequency
//! - the reading record — `works`, `lines`, `manual_sessions`, `anki_notes`,
//!   `word_days`, `lookups`
//! - the knowledge ledger ([`vocabulary`]) — one row per term: what I know,
//!   what I have met, and how often
//!
//! These belong together because term identity is **dictionary-gated**: "is this
//! token a word", "what is its canonical `(headword, reading)`", and "is it a
//! name rather than vocabulary" are all questions only the dictionaries can
//! answer, and every count worth keeping is keyed on the answer. Splitting them
//! into two files would mean the ledger could never join what it is keyed on.
//! See `spec/knowledge-db.md`.
//!
//! ## Who writes what
//!
//! read-stats writes the reading tables (line ingest, lookup capture, work
//! metadata) — it is not a pure reader, and saying so is the point of putting
//! them here rather than pretending they are private. yt-mine and manga-mine
//! read the dictionary tables. vn-mine's logger appends to `lines` directly.
//!
//! ## Where the queries live
//!
//! [`dictionaries`] is here because three tools call it. The reading tables are
//! defined here (the schema is shared, so it needs one owner) but queried from
//! read-stats, which is still their only consumer — moving those helpers up
//! before a second caller exists would be inventing an interface with no one to
//! test it against. When kotodex or the `#read` highlighter needs them, they
//! move, and the tables will not have to.

pub mod dictionaries;
pub mod vocabulary;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

const MIGRATION_DICT: &str = include_str!("../../migrations/knowledge/001_dictionaries.sql");
const MIGRATION_PITCH: &str = include_str!("../../migrations/knowledge/002_pitch.sql");
const MIGRATION_FREQ: &str = include_str!("../../migrations/knowledge/003_frequency.sql");
const MIGRATION_READING: &str = include_str!("../../migrations/knowledge/004_reading.sql");
const MIGRATION_VOCAB: &str = include_str!("../../migrations/knowledge/005_vocabulary.sql");

/// A connection pool for `knowledge.db`.
///
/// A newtype rather than a bare `SqlitePool` so that a program holding more
/// than one database — read-stats holds this and its own — cannot pass the
/// wrong one by accident. Both are `SqlitePool`, both are `create_if_missing`,
/// and the failure mode is silent: the query succeeds against a freshly created
/// empty table and the data appears to have vanished. The compiler can rule
/// that out for free, so it should.
#[derive(Clone, Debug)]
pub struct Knowledge(SqlitePool);

impl Knowledge {
    /// Open (creating if absent) and migrate.
    pub async fn open(db_path: &str) -> Result<Self, sqlx::Error> {
        let opts = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await?;
        // WAL + busy_timeout: vn-ws-logger.py appends to `lines` concurrently,
        // and yt-mine/manga-mine read the dictionaries from their own processes.
        sqlx::raw_sql("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .execute(&pool)
            .await?;

        let k = Knowledge(pool);
        k.migrate().await?;
        Ok(k)
    }

    /// Wrap an already-open pool. For tests and for callers that manage their
    /// own connection setup.
    pub fn from_pool(pool: SqlitePool) -> Self {
        Knowledge(pool)
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.0
    }

    /// Replay every migration. Each file is idempotent (`CREATE TABLE IF NOT
    /// EXISTS`), so there is no version table to keep in sync; what SQLite
    /// can't express idempotently is guarded by [`has_column`].
    async fn migrate(&self) -> Result<(), sqlx::Error> {
        for sql in [
            MIGRATION_DICT,
            MIGRATION_PITCH,
            MIGRATION_FREQ,
            MIGRATION_READING,
            MIGRATION_VOCAB,
        ] {
            sqlx::raw_sql(sql).execute(&self.0).await?;
        }

        // Replace old single-column indexes with the composite ones above.
        sqlx::raw_sql(
            "DROP INDEX IF EXISTS idx_dictionary_entries_term;\
             DROP INDEX IF EXISTS idx_dictionary_entries_dict;\
             DROP INDEX IF EXISTS idx_dictionary_pitch_term;\
             DROP INDEX IF EXISTS idx_dictionary_pitch_dict;",
        )
        .execute(&self.0)
        .await?;

        // Dictionaries imported before roles existed default to `reference`,
        // which is the safe answer: it counts for the wordhood gate and not
        // toward the vocabulary total.
        if !has_column(&self.0, "dictionaries", "role").await? {
            sqlx::raw_sql(&format!(
                "ALTER TABLE dictionaries ADD COLUMN role TEXT NOT NULL DEFAULT '{}'",
                dictionaries::Role::Reference.as_str()
            ))
            .execute(&self.0)
            .await?;
        }
        // `works` predates the per-work capture window.
        if !has_column(&self.0, "works", "vn_window").await? {
            sqlx::raw_sql("ALTER TABLE works ADD COLUMN vn_window TEXT")
                .execute(&self.0)
                .await?;
        }
        // `manual_sessions` predates pasting the text that was read. Rows
        // logged before it stay as they were: an estimated char count and no
        // content, which is exactly what they are.
        for column in ["content", "url"] {
            if !has_column(&self.0, "manual_sessions", column).await? {
                sqlx::raw_sql(&format!(
                    "ALTER TABLE manual_sessions ADD COLUMN {column} TEXT"
                ))
                .execute(&self.0)
                .await?;
            }
        }
        // `manual_sessions.end_ts` was NOT NULL when every session carried a
        // minute count. SQLite cannot drop a NOT NULL in place, so the table is
        // rebuilt — the one case in this file that needs more than an ALTER.
        // Existing rows keep their `end_ts`: they *were* timed, and a real
        // duration must never be replaced by an estimate.
        if column_is_not_null(&self.0, "manual_sessions", "end_ts").await? {
            sqlx::raw_sql(
                "BEGIN;\
                 CREATE TABLE manual_sessions_new (\
                     id INTEGER PRIMARY KEY, start_ts REAL NOT NULL, end_ts REAL,\
                     chars INTEGER NOT NULL, source TEXT NOT NULL DEFAULT 'book',\
                     work TEXT, pages REAL, note TEXT, content TEXT, url TEXT);\
                 INSERT INTO manual_sessions_new \
                     SELECT id, start_ts, end_ts, chars, source, work, pages, note, content, url \
                     FROM manual_sessions;\
                 DROP TABLE manual_sessions;\
                 ALTER TABLE manual_sessions_new RENAME TO manual_sessions;\
                 CREATE INDEX IF NOT EXISTS idx_manual_sessions_start_ts \
                     ON manual_sessions(start_ts);\
                 COMMIT;",
            )
            .execute(&self.0)
            .await?;
        }
        Ok(())
    }
}

/// Whether `table.column` is declared NOT NULL. Used to detect a schema that
/// predates a column being made optional, which SQLite can only fix by
/// rebuilding the table.
async fn column_is_not_null(
    pool: &SqlitePool,
    table: &str,
    column: &str,
) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().any(|r| {
        let name: &str = r.get("name");
        name == column && r.get::<i64, _>("notnull") != 0
    }))
}

pub async fn has_column(pool: &SqlitePool, table: &str, column: &str) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().any(|r| {
        let name: &str = r.get("name");
        name == column
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn temp_knowledge() -> Knowledge {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("jp-core-knowledge-{nanos}.db"));
        Knowledge::open(path.to_str().unwrap()).await.unwrap()
    }

    #[tokio::test]
    async fn migrations_are_idempotent_and_create_both_halves() {
        let k = temp_knowledge().await;
        // Running them again must not fail — this is what every startup does.
        k.migrate().await.unwrap();

        for table in [
            "dictionaries",
            "dictionary_entries",
            "dictionary_pitch",
            "dictionary_frequency",
            "works",
            "lines",
            "manual_sessions",
            "anki_notes",
            "word_days",
            "lookups",
            "vocabulary",
        ] {
            let count: (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(k.pool())
                .await
                .unwrap_or_else(|e| panic!("{table} missing: {e}"));
            assert_eq!(count.0, 0);
        }
    }

    #[tokio::test]
    async fn the_role_column_is_added_to_a_pre_role_database() {
        let k = temp_knowledge().await;
        // Simulate the old schema by dropping the column back off.
        sqlx::raw_sql("ALTER TABLE dictionaries DROP COLUMN role")
            .execute(k.pool())
            .await
            .unwrap();
        assert!(!has_column(k.pool(), "dictionaries", "role").await.unwrap());

        k.migrate().await.unwrap();
        assert!(has_column(k.pool(), "dictionaries", "role").await.unwrap());
    }
}
