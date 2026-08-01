//! `knowledge.db` — the shared database, and the handle that opens it.
//!
//! Holds what is about the language and what has been read, rather than about
//! one app's workflow. CLAUDE.md lists the contents; `spec/knowledge-db.md` has
//! the reasoning.
//!
//! One file because term identity is dictionary-gated — "is this a word", "what
//! is its `(headword, reading)`", "is it a name" — and every count is keyed on
//! that answer. Split in two, the ledger could not join what it is keyed on.
//!
//! [`dictionaries`] lives here because three tools call it. The reading tables
//! are defined here (shared schema needs one owner) but queried from read-stats,
//! their only consumer so far.

pub mod dictionaries;
pub mod lexeme;
pub mod vocabulary;
pub mod work_terms;

use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

const MIGRATION_DICT: &str = include_str!("../../migrations/knowledge/001_dictionaries.sql");
const MIGRATION_PITCH: &str = include_str!("../../migrations/knowledge/002_pitch.sql");
const MIGRATION_FREQ: &str = include_str!("../../migrations/knowledge/003_frequency.sql");
const MIGRATION_READING: &str = include_str!("../../migrations/knowledge/004_reading.sql");
const MIGRATION_VOCAB: &str = include_str!("../../migrations/knowledge/005_vocabulary.sql");
const MIGRATION_WORK_TERMS: &str = include_str!("../../migrations/knowledge/006_work_terms.sql");
const MIGRATION_LEXEME: &str = include_str!("../../migrations/knowledge/007_lexeme.sql");
const MIGRATION_VOCAB_HISTORY: &str =
    include_str!("../../migrations/knowledge/008_vocab_history.sql");

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
        // WAL + busy_timeout: vn-ws-logger.py appends to `lines` concurrently,
        // and yt-mine/manga-mine read the dictionaries from their own
        // processes.
        //
        // Both go on the *connect options*, not through a `PRAGMA` run against
        // the pool. `busy_timeout` is per connection: executing it once hands
        // it to whichever single connection served that statement and leaves
        // the other four at zero, so a write that lands on one of those fails
        // with SQLITE_BUSY the instant the logger holds the write lock instead
        // of waiting the five seconds it was supposed to. (`journal_mode` is
        // persisted in the file, so that half survived either way — which is
        // what made this look like it worked.)
        let opts = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
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
            MIGRATION_WORK_TERMS,
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
        // The lexeme layer. `sequence` is the dictionary's own entry id, the
        // only thing that says two spellings are one word; `seq_checked` marks
        // a cached dictionary whose zip has already been re-read for them, so
        // one that simply publishes none is not re-parsed every startup.
        if !has_column(&self.0, "dictionary_entries", "sequence").await? {
            sqlx::raw_sql("ALTER TABLE dictionary_entries ADD COLUMN sequence INTEGER")
                .execute(&self.0)
                .await?;
        }
        if !has_column(&self.0, "dictionaries", "seq_checked").await? {
            sqlx::raw_sql(
                "ALTER TABLE dictionaries ADD COLUMN seq_checked INTEGER NOT NULL DEFAULT 0",
            )
            .execute(&self.0)
            .await?;
        }
        // The master dictionary's escape hatch. Sankoku is a general-purpose
        // dictionary and does not carry domain vocabulary — 冪等性, 可用性 and
        // 復号 are absent, and so are their stems, so no decomposition rule
        // reaches them either. Swapping in JMdict would admit them at the cost
        // of every idiom and orthographic variant, which is the noise the
        // master role exists to keep out.
        //
        // So the dictionary keeps answering the dictionary's question
        // (`in_master`) and the reader answers theirs. A promoted term counts
        // toward the vocabulary scale though no master dictionary lists it.
        // Like `status`, it is an assertion: only a person sets it.
        if !has_column(&self.0, "vocabulary", "promoted").await? {
            sqlx::raw_sql("ALTER TABLE vocabulary ADD COLUMN promoted INTEGER NOT NULL DEFAULT 0")
                .execute(&self.0)
                .await?;
        }
        // The channel by which a write tells the history trigger which pass it
        // was. Nullable: a site that forgets it loses a label, not an event.
        if !has_column(&self.0, "vocabulary", "status_source").await? {
            sqlx::raw_sql("ALTER TABLE vocabulary ADD COLUMN status_source TEXT")
                .execute(&self.0)
                .await?;
        }
        // Runs after the ALTERs above, not in the loop: it indexes a column
        // they add.
        sqlx::raw_sql(MIGRATION_LEXEME).execute(&self.0).await?;
        // Likewise: its triggers read `promoted` and `status_source`.
        sqlx::raw_sql(MIGRATION_VOCAB_HISTORY)
            .execute(&self.0)
            .await?;
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

    /// The regression this exists for: `busy_timeout` is a per-connection
    /// setting, so setting it with a `PRAGMA` against the pool reached one
    /// connection and left the rest at zero — and a write that happened to
    /// land on one of those failed with "database is locked" the moment
    /// vn-ws-logger.py held the write lock, instead of waiting five seconds.
    #[tokio::test]
    async fn every_pooled_connection_waits_for_a_busy_database() {
        let k = temp_knowledge().await;
        // Hold several connections at once, so each answer comes from a
        // different one rather than the same connection five times over.
        let mut held = Vec::new();
        for _ in 0..5 {
            held.push(k.pool().acquire().await.unwrap());
        }
        for conn in held.iter_mut() {
            let (timeout,): (i64,) = sqlx::query_as("PRAGMA busy_timeout")
                .fetch_one(&mut **conn)
                .await
                .unwrap();
            assert_eq!(timeout, 5000, "every connection, not just the first");
            let (mode,): (String,) = sqlx::query_as("PRAGMA journal_mode")
                .fetch_one(&mut **conn)
                .await
                .unwrap();
            assert_eq!(mode, "wal");
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
