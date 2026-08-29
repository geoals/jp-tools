//! Opening the two databases, and the migrations kotodex-server owns.
//!
//! kotodex-server holds two: its own (`settings`, `reader_marks`,
//! `work_covers`) and jp-core's shared `knowledge.db` (`lines`, `works`,
//! `manual_sessions`, `anki_notes`, `word_days`, `lookups`, and the dictionary
//! cache). Only the first is migrated here — the shared schema has one owner,
//! and it is [`jp_core::knowledge`].
//!
//! Migrations are plain `.sql` files replayed unconditionally on every start —
//! each is written to be idempotent (`CREATE TABLE IF NOT EXISTS`), so there is
//! no version table to keep in sync.

use std::time::Duration;

use jp_core::knowledge::Knowledge;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

const MIGRATION_LOCAL: &str = include_str!("../../migrations/001_settings.sql");
const MIGRATION_READER_MARKS: &str = include_str!("../../migrations/002_reader_marks.sql");
const MIGRATION_WORK_COVERS: &str = include_str!("../../migrations/003_work_covers.sql");

/// The name this database had before the crate was called kotodex-server.
const FORMER_DB_NAME: &str = "read-stats.db";

/// Move a database left by an older release onto the current name.
///
/// Renamed rather than migrated because nothing inside the file changes: only
/// what it is called. The WAL and shared-memory files travel with it — SQLite
/// derives their names from the database's, so leaving them behind would strip
/// whatever had not been checkpointed.
///
/// Silent when there is nothing to move, which is every start after the first.
fn adopt_former_name(db_path: &str) {
    let path = std::path::Path::new(db_path);
    let Some(former) = path.parent().map(|d| d.join(FORMER_DB_NAME)) else {
        return;
    };
    // An existing database under the current name wins: a release that has
    // already run is the live one, and a stale file beside it is not an
    // upgrade to perform.
    if path.exists() || !former.exists() {
        return;
    }
    for suffix in ["", "-wal", "-shm"] {
        let from = with_suffix(&former, suffix);
        let to = with_suffix(path, suffix);
        if from.exists()
            && let Err(e) = std::fs::rename(&from, &to)
        {
            tracing::warn!(from = %from.display(), to = %to.display(), error = %e,
                "could not adopt the former database name");
            return;
        }
    }
    tracing::info!(former = %former.display(), now = %path.display(),
        "adopted the database from the read-stats name");
}

fn with_suffix(path: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    std::path::PathBuf::from(name)
}

/// Open kotodex-server's own database.
pub async fn create_pool(db_path: &str) -> Result<SqlitePool, sqlx::Error> {
    adopt_former_name(db_path);
    // WAL + busy_timeout, on the connect options rather than as a `PRAGMA`
    // against the pool: `busy_timeout` is per connection, so running it once
    // sets it on one of the five and leaves the rest at zero.
    jp_core::knowledge::ensure_parent_dir(db_path)?;
    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;
    sqlx::raw_sql(MIGRATION_LOCAL).execute(&pool).await?;
    sqlx::raw_sql(MIGRATION_READER_MARKS).execute(&pool).await?;
    sqlx::raw_sql(MIGRATION_WORK_COVERS).execute(&pool).await?;
    Ok(pool)
}

/// Open the shared knowledge database and bring the line stream up to date.
///
/// `recount` is off for the demo, whose seed is already consistent and whose
/// whole point is that a boot changes nothing.
pub async fn open_knowledge(db_path: &str, recount: bool) -> Result<Knowledge, sqlx::Error> {
    let knowledge = Knowledge::open(db_path).await?;
    if recount {
        recount_line_chars(&knowledge).await?;
    }
    Ok(knowledge)
}

/// Bring `lines.chars` in line with `jp_core::text::chars::count_chars`, which
/// excludes punctuation so the speeds are comparable with texthooker-ui's.
///
/// Unconditional rather than watermarked, so changing the counting rule is
/// enough to correct every row already stored. Only differing rows are written,
/// so once the column agrees this is a read-only scan.
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
