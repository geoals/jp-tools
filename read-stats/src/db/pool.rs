//! Opening the two databases, and the migrations read-stats owns.
//!
//! read-stats holds two: its own (`settings`, `pauses`, `reader_marks`,
//! `work_covers`) and jp-core's shared `knowledge.db` (`lines`, `works`,
//! `manual_sessions`, `anki_notes`, `word_days`, `lookups`, and the dictionary
//! cache). Only the first is migrated here — the shared schema has one owner,
//! and it is [`jp_core::knowledge`].
//!
//! Migrations are plain `.sql` files replayed unconditionally on every start —
//! each is written to be idempotent (`CREATE TABLE IF NOT EXISTS`), so there is
//! no version table to keep in sync.

use jp_core::knowledge::Knowledge;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

const MIGRATION_LOCAL: &str = include_str!("../../migrations/001_settings_and_pauses.sql");
const MIGRATION_READER_MARKS: &str = include_str!("../../migrations/002_reader_marks.sql");
const MIGRATION_WORK_COVERS: &str = include_str!("../../migrations/003_work_covers.sql");

/// Open read-stats' own database.
pub async fn create_pool(db_path: &str) -> Result<SqlitePool, sqlx::Error> {
    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;

    // WAL + busy_timeout: vn-ws-logger.py reads `settings` from this DB while
    // the server is writing to it.
    sqlx::raw_sql("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
        .execute(&pool)
        .await?;
    sqlx::raw_sql(MIGRATION_LOCAL).execute(&pool).await?;
    sqlx::raw_sql(MIGRATION_READER_MARKS).execute(&pool).await?;
    sqlx::raw_sql(MIGRATION_WORK_COVERS).execute(&pool).await?;
    Ok(pool)
}

/// Open the shared knowledge database and bring the line stream up to date.
pub async fn open_knowledge(db_path: &str) -> Result<Knowledge, sqlx::Error> {
    let knowledge = Knowledge::open(db_path).await?;
    recount_line_chars(&knowledge).await?;
    Ok(knowledge)
}

/// Refuse to run against a half-migrated pair of databases.
///
/// Both files are created on demand, so pointing at the wrong path does not
/// fail — it silently produces an empty `knowledge.db` and a dashboard showing
/// a lifetime of zero. The one shape that cannot be innocent is old rows still
/// sitting in the local database while the shared one is empty: that is a
/// migration which has not been run, and continuing would start a second
/// history beside the real one.
pub async fn check_migrated(local: &SqlitePool, knowledge: &Knowledge) -> Result<(), String> {
    let stale: Option<(i64,)> = sqlx::query_as("SELECT COUNT(*) FROM lines")
        .fetch_optional(local)
        .await
        .unwrap_or(None);
    // No `lines` table locally: already migrated, or a fresh install.
    let Some((stale_lines,)) = stale else {
        return Ok(());
    };
    if stale_lines == 0 {
        return Ok(());
    }
    let (migrated,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM lines")
        .fetch_one(knowledge.pool())
        .await
        .map_err(|e| format!("knowledge database unreadable: {e}"))?;
    if migrated > 0 {
        return Ok(());
    }
    Err(format!(
        "{stale_lines} lines are still in read-stats' own database, and knowledge.db is empty. \
         Run scripts/migrate-knowledge-db.sh (with the VN and vn-buffer stopped) first."
    ))
}

/// Bring `lines.chars` in line with `jp_core::text::chars::count_chars`, which
/// excludes punctuation; rows written under the old rule counted every
/// non-whitespace codepoint, inflating chars/h relative to texthooker-ui.
///
/// Deliberately unconditional rather than watermarked: vn-ws-logger.py writes
/// this column too, so a logger still running the old rule (it can't be
/// restarted while Textractor is attached) keeps producing rows that need
/// fixing. Only differing rows are written, so once both sides agree this is a
/// read-only scan.
async fn recount_line_chars(knowledge: &Knowledge) -> Result<(), sqlx::Error> {
    let pool = knowledge.pool();
    let rows = sqlx::query("SELECT id, chars, text FROM lines WHERE text IS NOT NULL")
        .fetch_all(pool)
        .await?;
    let updates: Vec<(i64, i64)> = rows
        .iter()
        .filter_map(|r| {
            let text: String = r.get("text");
            let recounted = jp_core::text::chars::count_chars(&text);
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
