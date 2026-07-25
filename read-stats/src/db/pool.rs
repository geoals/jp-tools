//! Opening the database, and the migrations it runs on the way up.
//!
//! Migrations are plain `.sql` files replayed unconditionally on every start —
//! each is written to be idempotent (`CREATE TABLE IF NOT EXISTS`), so there is
//! no version table to keep in sync. What SQLite can't express idempotently
//! (`ALTER TABLE ADD COLUMN`) is guarded by [`has_column`] below.

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

const MIGRATION: &str = include_str!("../../migrations/001_create_stats_tables.sql");
const MIGRATION_WORKS: &str = include_str!("../../migrations/002_create_works.sql");
const MIGRATION_ANKI: &str = include_str!("../../migrations/003_create_anki_tables.sql");
const MIGRATION_LOOKUPS: &str = include_str!("../../migrations/004_create_lookups.sql");
const MIGRATION_LOOKUP_IDX: &str = include_str!("../../migrations/005_create_lookup_indexes.sql");
const MIGRATION_READER_MARKS: &str = include_str!("../../migrations/006_create_reader_marks.sql");
const MIGRATION_WORK_COVERS: &str = include_str!("../../migrations/007_create_work_covers.sql");

pub async fn create_pool(db_path: &str) -> Result<SqlitePool, sqlx::Error> {
    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;

    // WAL + busy_timeout: vn-ws-logger.py writes to this DB concurrently.
    sqlx::raw_sql("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
        .execute(&pool)
        .await?;
    sqlx::raw_sql(MIGRATION).execute(&pool).await?;
    sqlx::raw_sql(MIGRATION_WORKS).execute(&pool).await?;
    sqlx::raw_sql(MIGRATION_ANKI).execute(&pool).await?;
    sqlx::raw_sql(MIGRATION_LOOKUPS).execute(&pool).await?;
    sqlx::raw_sql(MIGRATION_LOOKUP_IDX).execute(&pool).await?;
    sqlx::raw_sql(MIGRATION_READER_MARKS).execute(&pool).await?;
    sqlx::raw_sql(MIGRATION_WORK_COVERS).execute(&pool).await?;

    // ALTER TABLE ADD COLUMN has no IF NOT EXISTS in SQLite — DBs created
    // before the work column need it added.
    if !has_column(&pool, "lines", "work").await? {
        sqlx::raw_sql("ALTER TABLE lines ADD COLUMN work TEXT")
            .execute(&pool)
            .await?;
    }
    // Retroactively discarded lines (see `discard_lines`). Every read of the
    // stream filters on it, so it has to exist before anything queries.
    if !has_column(&pool, "lines", "discarded").await? {
        sqlx::raw_sql("ALTER TABLE lines ADD COLUMN discarded INTEGER NOT NULL DEFAULT 0")
            .execute(&pool)
            .await?;
    }
    // works briefly stored VNDB metadata; it's cover-only now.
    for col in ["vndb_id", "length_minutes"] {
        if has_column(&pool, "works", col).await? {
            sqlx::raw_sql(&format!("ALTER TABLE works DROP COLUMN {col}"))
                .execute(&pool)
                .await?;
        }
    }
    // The VN's window title is per-work now (each VN has its own), not one
    // global setting that goes stale the moment you switch VNs.
    if !has_column(&pool, "works", "vn_window").await? {
        sqlx::raw_sql("ALTER TABLE works ADD COLUMN vn_window TEXT")
            .execute(&pool)
            .await?;
    }
    recount_line_chars(&pool).await?;
    Ok(pool)
}

/// Bring `lines.chars` in line with `charcount::count_chars`, which excludes
/// punctuation; rows written under the old rule counted every non-whitespace
/// codepoint, inflating chars/h relative to texthooker-ui.
///
/// Deliberately unconditional rather than watermarked: vn-ws-logger.py writes
/// this column too, so a logger still running the old rule (it can't be
/// restarted while Textractor is attached) keeps producing rows that need
/// fixing. Only differing rows are written, so once both sides agree this is a
/// read-only scan.
async fn recount_line_chars(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let rows = sqlx::query("SELECT id, chars, text FROM lines WHERE text IS NOT NULL")
        .fetch_all(pool)
        .await?;
    let updates: Vec<(i64, i64)> = rows
        .iter()
        .filter_map(|r| {
            let text: String = r.get("text");
            let recounted = crate::charcount::count_chars(&text);
            (recounted != r.get::<i64, _>("chars")).then(|| (r.get("id"), recounted))
        })
        .collect();

    if updates.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for (id, chars) in &updates {
        sqlx::query("UPDATE lines SET chars = ? WHERE id = ?")
            .bind(chars)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    tracing::info!(
        scanned = rows.len(),
        updated = updates.len(),
        "recounted line chars (punctuation excluded)"
    );
    Ok(())
}

async fn has_column(pool: &SqlitePool, table: &str, column: &str) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().any(|r| {
        let name: &str = r.get("name");
        name == column
    }))
}
