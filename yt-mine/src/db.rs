use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

use crate::models::{Job, JobStatus, Sentence, TranscriptSegment};

const MIGRATION: &str = include_str!("../migrations/001_create_mining_tables.sql");

pub async fn create_pool(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    // WAL lets reads run during a write; busy_timeout is what stops a write
    // that collides with another one returning "database is locked" instantly.
    //
    // Both belong on the connect options. `busy_timeout` is a *per connection*
    // setting, so a `PRAGMA` executed against the pool reaches exactly one of
    // the five and leaves the others at zero — which is a lock error waiting
    // for the first moment two writers overlap.
    let opts = SqliteConnectOptions::from_str(database_url)?
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;

    sqlx::raw_sql(MIGRATION).execute(&pool).await?;

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

    if !has_column(&pool, "mining_sentences", "source").await? {
        sqlx::raw_sql(include_str!("../migrations/007_add_sentence_clips.sql"))
            .execute(&pool)
            .await?;
    }

    if !has_column(&pool, "mining_jobs", "refine_state").await? {
        sqlx::raw_sql(include_str!("../migrations/008_add_refine_state.sql"))
            .execute(&pool)
            .await?;
    }

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
        "SELECT id, youtube_url, video_id, video_title, audio_path, video_path, status, error_message, created_at, segments_found, video_duration, refine_state, refine_at FROM mining_jobs WHERE id = ?",
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
        "SELECT id, youtube_url, video_id, video_title, audio_path, video_path, status, error_message, created_at, segments_found, video_duration, refine_state, refine_at \
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
        "SELECT id, youtube_url, video_id, video_title, audio_path, video_path, status, error_message, created_at, segments_found, video_duration, refine_state, refine_at \
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
        refine_state: r.get("refine_state"),
        refine_at: r.get("refine_at"),
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

/// Where a line's text came from, and the window it can be cut out of.
#[derive(Debug, Clone, Default)]
pub struct SentenceOrigin<'a> {
    pub source: &'a str,
    pub clip: Option<&'a crate::services::clip::Clip>,
}

impl SentenceOrigin<'_> {
    pub fn captions() -> Self {
        Self {
            source: "captions",
            clip: None,
        }
    }
}

/// The title and duration, learned from the captions pass — which downloads
/// nothing, so there are no paths to record with them.
pub async fn update_job_title(
    pool: &SqlitePool,
    id: i64,
    video_title: &str,
    video_duration: Option<f64>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE mining_jobs SET video_title = ?, video_duration = ? WHERE id = ?")
        .bind(video_title)
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
    origin: &SentenceOrigin<'_>,
) -> Result<(), sqlx::Error> {
    for seg in segments {
        insert_sentence(pool, job_id, seg, origin).await?;
    }
    Ok(())
}

pub async fn insert_sentence(
    pool: &SqlitePool,
    job_id: i64,
    segment: &TranscriptSegment,
    origin: &SentenceOrigin<'_>,
) -> Result<(), sqlx::Error> {
    let now = chrono_now();
    sqlx::query(
        "INSERT INTO mining_sentences (job_id, text, start_time, end_time, created_at, source, clip_path, clip_audio_path, clip_start) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(job_id)
    .bind(&segment.text)
    .bind(segment.start)
    .bind(segment.end)
    .bind(&now)
    .bind(origin.source)
    .bind(origin.clip.map(|c| c.video_path.as_str()))
    .bind(origin.clip.map(|c| c.audio_path.as_str()))
    .bind(origin.clip.map(|c| c.start))
    .execute(pool)
    .await?;
    Ok(())
}

/// Attach a window to a line that had none — an export fetched one for a
/// caption line, and the next export or replay should reuse it.
pub async fn attach_clip(
    pool: &SqlitePool,
    sentence_id: i64,
    clip: &crate::services::clip::Clip,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE mining_sentences SET clip_path = ?, clip_audio_path = ?, clip_start = ? WHERE id = ?",
    )
    .bind(&clip.video_path)
    .bind(&clip.audio_path)
    .bind(clip.start)
    .bind(sentence_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Drop the caption lines a whisper pass is about to replace.
///
/// Only caption lines: a window sharpened twice, or overlapping one already
/// sharpened, must not throw away the better transcript it already has.
pub async fn delete_caption_sentences_in_window(
    pool: &SqlitePool,
    job_id: i64,
    start: f64,
    end: f64,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM mining_sentences \
         WHERE job_id = ? AND source = 'captions' AND start_time < ? AND end_time > ?",
    )
    .bind(job_id)
    .bind(end)
    .bind(start)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub async fn set_refine_state(
    pool: &SqlitePool,
    job_id: i64,
    state: Option<&str>,
    at: Option<f64>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE mining_jobs SET refine_state = ?, refine_at = ? WHERE id = ?")
        .bind(state)
        .bind(at)
        .bind(job_id)
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
        "SELECT id, job_id, text, start_time, end_time, created_at, source, clip_path, clip_audio_path, clip_start FROM mining_sentences WHERE job_id = ? ORDER BY start_time",
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
            source: r.get("source"),
            clip_path: r.get("clip_path"),
            clip_audio_path: r.get("clip_audio_path"),
            clip_start: r.get("clip_start"),
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
        "SELECT id, job_id, text, start_time, end_time, created_at, source, clip_path, clip_audio_path, clip_start FROM mining_sentences WHERE id IN ({}) ORDER BY start_time",
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
            source: r.get("source"),
            clip_path: r.get("clip_path"),
            clip_audio_path: r.get("clip_audio_path"),
            clip_start: r.get("clip_start"),
        })
        .collect())
}

/// Delete jobs that were left in a non-terminal state (pending/downloading/transcribing)
/// from a previous run, along with any partial sentences they accumulated.
/// Returns the number of deleted jobs.
pub async fn delete_incomplete_jobs(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let statuses = [
        JobStatus::Pending.as_str(),
        JobStatus::Fetching.as_str(),
        JobStatus::Downloading.as_str(),
        JobStatus::Transcribing.as_str(),
    ];

    sqlx::query(
        "DELETE FROM mining_sentences WHERE job_id IN \
         (SELECT id FROM mining_jobs WHERE status IN (?, ?, ?, ?))",
    )
    .bind(statuses[0])
    .bind(statuses[1])
    .bind(statuses[2])
    .bind(statuses[3])
    .execute(pool)
    .await?;

    let result = sqlx::query("DELETE FROM mining_jobs WHERE status IN (?, ?, ?, ?)")
        .bind(statuses[0])
        .bind(statuses[1])
        .bind(statuses[2])
        .bind(statuses[3])
        .execute(pool)
        .await?;

    Ok(result.rows_affected())
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
        // 004 uses ALTER TABLE ADD COLUMN which would fail without the guard.
        assert!(has_column(&pool, "mining_jobs", "video_id").await.unwrap());
    }

    #[tokio::test]
    async fn create_and_get_job() {
        let pool = test_pool().await;

        let id = create_job(
            &pool,
            "https://youtube.com/watch?v=dQw4w9WgXcQ",
            "dQw4w9WgXcQ",
        )
        .await
        .unwrap();
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
        let id = create_job(
            &pool,
            "https://youtube.com/watch?v=dQw4w9WgXcQ",
            "dQw4w9WgXcQ",
        )
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
        let err_id = create_job(
            &pool,
            "https://youtube.com/watch?v=dQw4w9WgXcQ",
            "dQw4w9WgXcQ",
        )
        .await
        .unwrap();
        update_job_status(&pool, err_id, &JobStatus::Error, Some("failed"))
            .await
            .unwrap();

        let ok_id = create_job(
            &pool,
            "https://youtube.com/watch?v=dQw4w9WgXcQ",
            "dQw4w9WgXcQ",
        )
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
        let id = create_job(
            &pool,
            "https://youtube.com/watch?v=dQw4w9WgXcQ",
            "dQw4w9WgXcQ",
        )
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
        let id = create_job(
            &pool,
            "https://youtube.com/watch?v=dQw4w9WgXcQ",
            "dQw4w9WgXcQ",
        )
        .await
        .unwrap();

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
        let id = create_job(
            &pool,
            "https://youtube.com/watch?v=dQw4w9WgXcQ",
            "dQw4w9WgXcQ",
        )
        .await
        .unwrap();

        update_job_download(
            &pool,
            id,
            "/tmp/audio.wav",
            "Test Video",
            "/tmp/video.mp4",
            Some(120.5),
        )
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
        let job_id = create_job(
            &pool,
            "https://youtube.com/watch?v=dQw4w9WgXcQ",
            "dQw4w9WgXcQ",
        )
        .await
        .unwrap();

        let segments = vec![
            TranscriptSegment {
                start: 0.0,
                end: 3.2,
                text: "First sentence".into(),
            },
            TranscriptSegment {
                start: 3.5,
                end: 6.1,
                text: "Second sentence".into(),
            },
        ];

        insert_sentences(&pool, job_id, &segments, &SentenceOrigin::captions())
            .await
            .unwrap();

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
        let job_id = create_job(
            &pool,
            "https://youtube.com/watch?v=dQw4w9WgXcQ",
            "dQw4w9WgXcQ",
        )
        .await
        .unwrap();

        let segments = vec![
            TranscriptSegment {
                start: 0.0,
                end: 1.0,
                text: "A".into(),
            },
            TranscriptSegment {
                start: 1.0,
                end: 2.0,
                text: "B".into(),
            },
            TranscriptSegment {
                start: 2.0,
                end: 3.0,
                text: "C".into(),
            },
        ];
        insert_sentences(&pool, job_id, &segments, &SentenceOrigin::captions())
            .await
            .unwrap();

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
        let seg = TranscriptSegment {
            start: 0.0,
            end: 1.0,
            text: "partial".into(),
        };
        insert_sentences(
            &pool,
            transcribing,
            &[seg.clone()],
            &SentenceOrigin::captions(),
        )
        .await
        .unwrap();
        // And to the done job (should survive)
        insert_sentences(&pool, done, &[seg], &SentenceOrigin::captions())
            .await
            .unwrap();

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
}
