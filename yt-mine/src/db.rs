use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};

use crate::models::{Job, JobStatus, Sentence, TranscriptSegment, VocabStatus};

const MIGRATION: &str = include_str!("../migrations/001_create_mining_tables.sql");
const VOCAB_MIGRATION: &str = include_str!("../migrations/007_create_vocabulary_table.sql");

pub async fn create_pool(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await?;

    // WAL mode allows concurrent reads during writes (no reader-blocks-writer).
    // busy_timeout prevents "database is locked" errors under contention.
    sqlx::raw_sql("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
        .execute(&pool)
        .await?;

    sqlx::raw_sql(MIGRATION).execute(&pool).await?;
    jp_core::db::run_migrations(&pool).await?;

    // ALTER TABLE ADD COLUMN has no IF NOT EXISTS in SQLite,
    // so check whether the column is already present first.
    if !has_column(&pool, "mining_jobs", "video_id").await? {
        sqlx::raw_sql(include_str!("../migrations/004_add_video_id.sql"))
            .execute(&pool)
            .await?;
    }

    if !has_column(&pool, "mining_jobs", "segments_found").await? {
        sqlx::raw_sql(include_str!("../migrations/005_add_segments_found.sql"))
            .execute(&pool)
            .await?;
    }

    if !has_column(&pool, "mining_jobs", "video_duration").await? {
        sqlx::raw_sql(include_str!("../migrations/006_add_video_duration.sql"))
            .execute(&pool)
            .await?;
    }

    sqlx::raw_sql(VOCAB_MIGRATION).execute(&pool).await?;

    Ok(pool)
}

/// Check whether a table already has a given column.
async fn has_column(pool: &SqlitePool, table: &str, column: &str) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().any(|r| {
        let name: &str = r.get("name");
        name == column
    }))
}

pub async fn create_job(
    pool: &SqlitePool,
    youtube_url: &str,
    video_id: &str,
) -> Result<i64, sqlx::Error> {
    let now = chrono_now();
    let row = sqlx::query(
        "INSERT INTO mining_jobs (youtube_url, video_id, status, created_at) VALUES (?, ?, 'pending', ?) RETURNING id",
    )
    .bind(youtube_url)
    .bind(video_id)
    .bind(&now)
    .fetch_one(pool)
    .await?;

    Ok(row.get("id"))
}

pub async fn get_job(pool: &SqlitePool, id: i64) -> Result<Option<Job>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT id, youtube_url, video_id, video_title, audio_path, video_path, status, error_message, created_at, segments_found, video_duration FROM mining_jobs WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(job_from_row))
}

/// Find the most recent job for a video ID, including error jobs.
///
/// Used for the video page display — shows the current state even if it errored.
pub async fn get_latest_job_by_video_id(
    pool: &SqlitePool,
    video_id: &str,
) -> Result<Option<Job>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT id, youtube_url, video_id, video_title, audio_path, video_path, status, error_message, created_at, segments_found, video_duration \
         FROM mining_jobs \
         WHERE video_id = ? \
         ORDER BY id DESC \
         LIMIT 1",
    )
    .bind(video_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(job_from_row))
}

/// Find the most recent non-error job for a video ID.
///
/// Returns `None` if no usable job exists (allowing callers to create a new one).
/// Error jobs are skipped so that re-submitting a failed video triggers a retry.
pub async fn get_job_by_video_id(
    pool: &SqlitePool,
    video_id: &str,
) -> Result<Option<Job>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT id, youtube_url, video_id, video_title, audio_path, video_path, status, error_message, created_at, segments_found, video_duration \
         FROM mining_jobs \
         WHERE video_id = ? AND status != 'error' \
         ORDER BY id DESC \
         LIMIT 1",
    )
    .bind(video_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(job_from_row))
}

fn job_from_row(r: sqlx::sqlite::SqliteRow) -> Job {
    let status_str: String = r.get("status");
    Job {
        id: r.get("id"),
        youtube_url: r.get("youtube_url"),
        video_id: r.get("video_id"),
        video_title: r.get("video_title"),
        audio_path: r.get("audio_path"),
        video_path: r.get("video_path"),
        status: JobStatus::from_str(&status_str).unwrap_or(JobStatus::Error),
        error_message: r.get("error_message"),
        created_at: r.get("created_at"),
        segments_found: r.get("segments_found"),
        video_duration: r.get("video_duration"),
    }
}

pub async fn update_job_status(
    pool: &SqlitePool,
    id: i64,
    status: &JobStatus,
    error_message: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE mining_jobs SET status = ?, error_message = ? WHERE id = ?")
        .bind(status.as_str())
        .bind(error_message)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_job_download(
    pool: &SqlitePool,
    id: i64,
    audio_path: &str,
    video_title: &str,
    video_path: &str,
    video_duration: Option<f64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE mining_jobs SET audio_path = ?, video_title = ?, video_path = ?, video_duration = ? WHERE id = ?",
    )
    .bind(audio_path)
    .bind(video_title)
    .bind(video_path)
    .bind(video_duration)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn insert_sentences(
    pool: &SqlitePool,
    job_id: i64,
    segments: &[TranscriptSegment],
) -> Result<(), sqlx::Error> {
    let now = chrono_now();
    for seg in segments {
        sqlx::query(
            "INSERT INTO mining_sentences (job_id, text, start_time, end_time, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(job_id)
        .bind(&seg.text)
        .bind(seg.start)
        .bind(seg.end)
        .bind(&now)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn insert_sentence(
    pool: &SqlitePool,
    job_id: i64,
    segment: &TranscriptSegment,
) -> Result<(), sqlx::Error> {
    let now = chrono_now();
    sqlx::query(
        "INSERT INTO mining_sentences (job_id, text, start_time, end_time, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(job_id)
    .bind(&segment.text)
    .bind(segment.start)
    .bind(segment.end)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_job_progress(
    pool: &SqlitePool,
    id: i64,
    segments_found: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE mining_jobs SET segments_found = ? WHERE id = ?")
        .bind(segments_found)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn count_sentences_for_job(pool: &SqlitePool, job_id: i64) -> Result<i64, sqlx::Error> {
    let row = sqlx::query("SELECT COUNT(*) as cnt FROM mining_sentences WHERE job_id = ?")
        .bind(job_id)
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>("cnt"))
}

pub async fn get_sentences_for_job(
    pool: &SqlitePool,
    job_id: i64,
) -> Result<Vec<Sentence>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, job_id, text, start_time, end_time, created_at FROM mining_sentences WHERE job_id = ? ORDER BY start_time",
    )
    .bind(job_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| Sentence {
            id: r.get("id"),
            job_id: r.get("job_id"),
            text: r.get("text"),
            start_time: r.get("start_time"),
            end_time: r.get("end_time"),
            created_at: r.get("created_at"),
        })
        .collect())
}

pub async fn get_sentences_by_ids(
    pool: &SqlitePool,
    ids: &[i64],
) -> Result<Vec<Sentence>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(vec![]);
    }

    // Build a query with placeholders for each ID
    let placeholders: Vec<&str> = ids.iter().map(|_| "?").collect();
    let query_str = format!(
        "SELECT id, job_id, text, start_time, end_time, created_at FROM mining_sentences WHERE id IN ({}) ORDER BY start_time",
        placeholders.join(", ")
    );

    let mut query = sqlx::query(&query_str);
    for id in ids {
        query = query.bind(id);
    }

    let rows = query.fetch_all(pool).await?;

    Ok(rows
        .into_iter()
        .map(|r| Sentence {
            id: r.get("id"),
            job_id: r.get("job_id"),
            text: r.get("text"),
            start_time: r.get("start_time"),
            end_time: r.get("end_time"),
            created_at: r.get("created_at"),
        })
        .collect())
}

/// Delete jobs that were left in a non-terminal state (pending/downloading/transcribing)
/// from a previous run, along with any partial sentences they accumulated.
/// Returns the number of deleted jobs.
pub async fn delete_incomplete_jobs(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let statuses = [
        JobStatus::Pending.as_str(),
        JobStatus::Downloading.as_str(),
        JobStatus::Transcribing.as_str(),
    ];

    sqlx::query(
        "DELETE FROM mining_sentences WHERE job_id IN \
         (SELECT id FROM mining_jobs WHERE status IN (?, ?, ?))",
    )
    .bind(statuses[0])
    .bind(statuses[1])
    .bind(statuses[2])
    .execute(pool)
    .await?;

    let result = sqlx::query("DELETE FROM mining_jobs WHERE status IN (?, ?, ?)")
        .bind(statuses[0])
        .bind(statuses[1])
        .bind(statuses[2])
        .execute(pool)
        .await?;

    Ok(result.rows_affected())
}

// --- Vocabulary ---

#[derive(Debug, Clone)]
pub struct VocabRow {
    pub lemma: String,
    pub reading: String,
    pub pos: Option<String>,
    pub status: VocabStatus,
    pub encounter_count: i64,
}

pub struct VocabUpsert {
    pub user_id: i64,
    pub lemma: String,
    pub reading: String,
    pub pos: Option<String>,
    pub status: VocabStatus,
    pub count: i64,
    pub source: String,
}

pub async fn get_vocab_by_lemmas(
    pool: &SqlitePool,
    user_id: i64,
    lemmas: &[String],
) -> Result<Vec<VocabRow>, sqlx::Error> {
    if lemmas.is_empty() {
        return Ok(vec![]);
    }

    let placeholders: Vec<&str> = lemmas.iter().map(|_| "?").collect();
    let query_str = format!(
        "SELECT lemma, reading, pos, status, encounter_count \
         FROM vocabulary WHERE user_id = ? AND lemma IN ({})",
        placeholders.join(", ")
    );

    let mut query = sqlx::query(&query_str).bind(user_id);
    for lemma in lemmas {
        query = query.bind(lemma);
    }

    let rows = query.fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let status_str: String = r.get("status");
            VocabRow {
                lemma: r.get("lemma"),
                reading: r.get("reading"),
                pos: r.get("pos"),
                status: VocabStatus::from_str(&status_str).unwrap_or(VocabStatus::Seen),
                encounter_count: r.get("encounter_count"),
            }
        })
        .collect())
}

pub async fn upsert_vocab_entries(
    pool: &SqlitePool,
    entries: &[VocabUpsert],
) -> Result<usize, sqlx::Error> {
    if entries.is_empty() {
        return Ok(0);
    }

    let now = chrono_now();
    let mut tx = pool.begin().await?;
    let mut count = 0usize;

    for entry in entries {
        let result = sqlx::query(
            "INSERT INTO vocabulary (user_id, lemma, reading, pos, status, encounter_count, first_seen, last_seen, source) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(user_id, lemma, reading) DO UPDATE SET \
               encounter_count = vocabulary.encounter_count + excluded.encounter_count, \
               last_seen = excluded.last_seen, \
               status = excluded.status",
        )
        .bind(entry.user_id)
        .bind(&entry.lemma)
        .bind(&entry.reading)
        .bind(&entry.pos)
        .bind(entry.status.as_str())
        .bind(entry.count)
        .bind(&now)
        .bind(&now)
        .bind(&entry.source)
        .execute(&mut *tx)
        .await?;

        count += result.rows_affected() as usize;
    }

    tx.commit().await?;
    Ok(count)
}

pub async fn get_blacklisted_keys(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<std::collections::HashSet<(String, String)>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT lemma, reading FROM vocabulary WHERE user_id = ? AND status = 'blacklisted'",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| (r.get("lemma"), r.get("reading")))
        .collect())
}

fn chrono_now() -> String {
    // ISO 8601 timestamp without external chrono dependency
    // In production this would use a proper time library, but for MVP
    // we use a simple approach that's testable
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| format!("{}", d.as_secs()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> SqlitePool {
        create_pool("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn migration_creates_tables() {
        let pool = test_pool().await;

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM mining_jobs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0);

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM mining_sentences")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0);
    }

    #[tokio::test]
    async fn migration_is_idempotent() {
        // Running create_pool twice on the same database must not fail.
        let pool = create_pool("sqlite::memory:").await.unwrap();
        // Re-run all migrations (simulates second server start).
        sqlx::raw_sql(MIGRATION).execute(&pool).await.unwrap();
        jp_core::db::run_migrations(&pool).await.unwrap();
        // 004 uses ALTER TABLE ADD COLUMN which would fail without the guard.
        assert!(has_column(&pool, "mining_jobs", "video_id").await.unwrap());
    }

    #[tokio::test]
    async fn create_and_get_job() {
        let pool = test_pool().await;

        let id = create_job(&pool, "https://youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ").await.unwrap();
        let job = get_job(&pool, id).await.unwrap().unwrap();

        assert_eq!(job.youtube_url, "https://youtube.com/watch?v=dQw4w9WgXcQ");
        assert_eq!(job.status, JobStatus::Pending);
        assert!(job.video_title.is_none());
        assert!(job.audio_path.is_none());
        assert!(job.error_message.is_none());
    }

    #[tokio::test]
    async fn get_job_returns_none_for_missing() {
        let pool = test_pool().await;
        let job = get_job(&pool, 999).await.unwrap();
        assert!(job.is_none());
    }

    #[tokio::test]
    async fn get_job_by_video_id_returns_none_when_no_jobs() {
        let pool = test_pool().await;
        let job = get_job_by_video_id(&pool, "dQw4w9WgXcQ").await.unwrap();
        assert!(job.is_none());
    }

    #[tokio::test]
    async fn get_job_by_video_id_finds_done_job() {
        let pool = test_pool().await;
        let id = create_job(&pool, "https://youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ")
            .await
            .unwrap();
        update_job_status(&pool, id, &JobStatus::Done, None)
            .await
            .unwrap();

        let job = get_job_by_video_id(&pool, "dQw4w9WgXcQ")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.id, id);
        assert_eq!(job.status, JobStatus::Done);
    }

    #[tokio::test]
    async fn get_job_by_video_id_skips_error_jobs() {
        let pool = test_pool().await;

        // Create an error job, then a done job
        let err_id = create_job(&pool, "https://youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ")
            .await
            .unwrap();
        update_job_status(&pool, err_id, &JobStatus::Error, Some("failed"))
            .await
            .unwrap();

        let ok_id = create_job(&pool, "https://youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ")
            .await
            .unwrap();
        update_job_status(&pool, ok_id, &JobStatus::Done, None)
            .await
            .unwrap();

        let job = get_job_by_video_id(&pool, "dQw4w9WgXcQ")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.id, ok_id);
    }

    #[tokio::test]
    async fn get_job_by_video_id_returns_none_when_only_errors() {
        let pool = test_pool().await;
        let id = create_job(&pool, "https://youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ")
            .await
            .unwrap();
        update_job_status(&pool, id, &JobStatus::Error, Some("failed"))
            .await
            .unwrap();

        let job = get_job_by_video_id(&pool, "dQw4w9WgXcQ").await.unwrap();
        assert!(job.is_none());
    }

    #[tokio::test]
    async fn update_job_status_sets_status_and_error() {
        let pool = test_pool().await;
        let id = create_job(&pool, "https://youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ").await.unwrap();

        update_job_status(&pool, id, &JobStatus::Downloading, None)
            .await
            .unwrap();
        let job = get_job(&pool, id).await.unwrap().unwrap();
        assert_eq!(job.status, JobStatus::Downloading);
        assert!(job.error_message.is_none());

        update_job_status(&pool, id, &JobStatus::Error, Some("download failed"))
            .await
            .unwrap();
        let job = get_job(&pool, id).await.unwrap().unwrap();
        assert_eq!(job.status, JobStatus::Error);
        assert_eq!(job.error_message.as_deref(), Some("download failed"));
    }

    #[tokio::test]
    async fn update_job_download_sets_audio_path_and_title() {
        let pool = test_pool().await;
        let id = create_job(&pool, "https://youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ").await.unwrap();

        update_job_download(&pool, id, "/tmp/audio.wav", "Test Video", "/tmp/video.mp4", Some(120.5))
            .await
            .unwrap();
        let job = get_job(&pool, id).await.unwrap().unwrap();
        assert_eq!(job.audio_path.as_deref(), Some("/tmp/audio.wav"));
        assert_eq!(job.video_title.as_deref(), Some("Test Video"));
        assert_eq!(job.video_duration, Some(120.5));
    }

    #[tokio::test]
    async fn insert_and_get_sentences() {
        let pool = test_pool().await;
        let job_id = create_job(&pool, "https://youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ").await.unwrap();

        let segments = vec![
            TranscriptSegment { start: 0.0, end: 3.2, text: "First sentence".into() },
            TranscriptSegment { start: 3.5, end: 6.1, text: "Second sentence".into() },
        ];

        insert_sentences(&pool, job_id, &segments).await.unwrap();

        let sentences = get_sentences_for_job(&pool, job_id).await.unwrap();
        assert_eq!(sentences.len(), 2);
        assert_eq!(sentences[0].text, "First sentence");
        assert_eq!(sentences[0].start_time, 0.0);
        assert_eq!(sentences[0].end_time, 3.2);
        assert_eq!(sentences[1].text, "Second sentence");
        assert_eq!(sentences[1].start_time, 3.5);
    }

    #[tokio::test]
    async fn get_sentences_by_ids_returns_matching() {
        let pool = test_pool().await;
        let job_id = create_job(&pool, "https://youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ").await.unwrap();

        let segments = vec![
            TranscriptSegment { start: 0.0, end: 1.0, text: "A".into() },
            TranscriptSegment { start: 1.0, end: 2.0, text: "B".into() },
            TranscriptSegment { start: 2.0, end: 3.0, text: "C".into() },
        ];
        insert_sentences(&pool, job_id, &segments).await.unwrap();

        let all = get_sentences_for_job(&pool, job_id).await.unwrap();
        let ids = vec![all[0].id, all[2].id];

        let selected = get_sentences_by_ids(&pool, &ids).await.unwrap();
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].text, "A");
        assert_eq!(selected[1].text, "C");
    }

    #[tokio::test]
    async fn get_sentences_by_ids_empty_returns_empty() {
        let pool = test_pool().await;
        let result = get_sentences_by_ids(&pool, &[]).await.unwrap();
        assert!(result.is_empty());
    }

    // --- delete incomplete jobs ---

    #[tokio::test]
    async fn delete_incomplete_jobs_removes_stale_jobs_and_sentences() {
        let pool = test_pool().await;
        let url = "https://youtube.com/watch?v=abc";

        // Create jobs in each status
        let pending = create_job(&pool, url, "pending1").await.unwrap();

        let downloading = create_job(&pool, url, "downloading1").await.unwrap();
        update_job_status(&pool, downloading, &JobStatus::Downloading, None)
            .await
            .unwrap();

        let transcribing = create_job(&pool, url, "transcribing1").await.unwrap();
        update_job_status(&pool, transcribing, &JobStatus::Transcribing, None)
            .await
            .unwrap();

        let done = create_job(&pool, url, "done1").await.unwrap();
        update_job_status(&pool, done, &JobStatus::Done, None)
            .await
            .unwrap();

        let error = create_job(&pool, url, "error1").await.unwrap();
        update_job_status(&pool, error, &JobStatus::Error, Some("fail"))
            .await
            .unwrap();

        // Add sentences to the transcribing job (partial data)
        let seg = TranscriptSegment { start: 0.0, end: 1.0, text: "partial".into() };
        insert_sentences(&pool, transcribing, &[seg.clone()]).await.unwrap();
        // And to the done job (should survive)
        insert_sentences(&pool, done, &[seg]).await.unwrap();

        let deleted = delete_incomplete_jobs(&pool).await.unwrap();
        assert_eq!(deleted, 3);

        // Incomplete jobs are gone
        assert!(get_job(&pool, pending).await.unwrap().is_none());
        assert!(get_job(&pool, downloading).await.unwrap().is_none());
        assert!(get_job(&pool, transcribing).await.unwrap().is_none());

        // Terminal jobs survive
        assert!(get_job(&pool, done).await.unwrap().is_some());
        assert!(get_job(&pool, error).await.unwrap().is_some());

        // Partial sentences for transcribing job are deleted
        let sentences = get_sentences_for_job(&pool, transcribing).await.unwrap();
        assert!(sentences.is_empty());

        // Sentences for done job survive
        let sentences = get_sentences_for_job(&pool, done).await.unwrap();
        assert_eq!(sentences.len(), 1);
    }

    #[tokio::test]
    async fn delete_incomplete_jobs_returns_zero_when_nothing_to_clean() {
        let pool = test_pool().await;
        let deleted = delete_incomplete_jobs(&pool).await.unwrap();
        assert_eq!(deleted, 0);
    }

    // --- vocabulary ---

    #[tokio::test]
    async fn vocab_migration_creates_table() {
        let pool = test_pool().await;
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM vocabulary")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0);
    }

    #[tokio::test]
    async fn upsert_vocab_inserts_new_entries() {
        let pool = test_pool().await;
        let entries = vec![
            VocabUpsert {
                user_id: 1,
                lemma: "東京".into(),
                reading: "トウキョウ".into(),
                pos: Some("名詞".into()),
                status: VocabStatus::Known,
                count: 2,
                source: "calibration".into(),
            },
        ];

        let count = upsert_vocab_entries(&pool, &entries).await.unwrap();
        assert_eq!(count, 1);

        let rows = get_vocab_by_lemmas(&pool, 1, &["東京".into()]).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].lemma, "東京");
        assert_eq!(rows[0].reading, "トウキョウ");
        assert_eq!(rows[0].status, VocabStatus::Known);
        assert_eq!(rows[0].encounter_count, 2);
    }

    #[tokio::test]
    async fn upsert_vocab_increments_encounter_count() {
        let pool = test_pool().await;

        let entries = vec![VocabUpsert {
            user_id: 1,
            lemma: "行く".into(),
            reading: "イク".into(),
            pos: Some("動詞".into()),
            status: VocabStatus::Seen,
            count: 3,
            source: "calibration".into(),
        }];
        upsert_vocab_entries(&pool, &entries).await.unwrap();

        // Upsert again with count 2 and status change
        let entries2 = vec![VocabUpsert {
            user_id: 1,
            lemma: "行く".into(),
            reading: "イク".into(),
            pos: Some("動詞".into()),
            status: VocabStatus::Known,
            count: 2,
            source: "calibration".into(),
        }];
        upsert_vocab_entries(&pool, &entries2).await.unwrap();

        let rows = get_vocab_by_lemmas(&pool, 1, &["行く".into()]).await.unwrap();
        assert_eq!(rows[0].encounter_count, 5); // 3 + 2
        assert_eq!(rows[0].status, VocabStatus::Known);
    }

    #[tokio::test]
    async fn get_vocab_by_lemmas_empty_returns_empty() {
        let pool = test_pool().await;
        let rows = get_vocab_by_lemmas(&pool, 1, &[]).await.unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn get_blacklisted_keys_returns_only_blacklisted() {
        let pool = test_pool().await;

        let entries = vec![
            VocabUpsert {
                user_id: 1,
                lemma: "する".into(),
                reading: "スル".into(),
                pos: Some("動詞".into()),
                status: VocabStatus::Blacklisted,
                count: 1,
                source: "calibration".into(),
            },
            VocabUpsert {
                user_id: 1,
                lemma: "東京".into(),
                reading: "トウキョウ".into(),
                pos: Some("名詞".into()),
                status: VocabStatus::Known,
                count: 1,
                source: "calibration".into(),
            },
        ];
        upsert_vocab_entries(&pool, &entries).await.unwrap();

        let blacklisted = get_blacklisted_keys(&pool, 1).await.unwrap();
        assert_eq!(blacklisted.len(), 1);
        assert!(blacklisted.contains(&("する".into(), "スル".into())));
    }

    #[tokio::test]
    async fn upsert_vocab_empty_entries_returns_zero() {
        let pool = test_pool().await;
        let count = upsert_vocab_entries(&pool, &[]).await.unwrap();
        assert_eq!(count, 0);
    }
}
