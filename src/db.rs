use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};

use crate::models::{Job, JobStatus, Sentence, TranscriptSegment};

const MIGRATION: &str = include_str!("../migrations/001_create_mining_tables.sql");

pub async fn create_pool(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await?;

    sqlx::raw_sql(MIGRATION).execute(&pool).await?;

    Ok(pool)
}

pub async fn create_job(pool: &SqlitePool, youtube_url: &str) -> Result<i64, sqlx::Error> {
    let now = chrono_now();
    let row = sqlx::query(
        "INSERT INTO mining_jobs (youtube_url, status, created_at) VALUES (?, 'pending', ?) RETURNING id",
    )
    .bind(youtube_url)
    .bind(&now)
    .fetch_one(pool)
    .await?;

    Ok(row.get("id"))
}

pub async fn get_job(pool: &SqlitePool, id: i64) -> Result<Option<Job>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT id, youtube_url, video_title, audio_path, video_path, status, error_message, created_at FROM mining_jobs WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| {
        let status_str: String = r.get("status");
        Job {
            id: r.get("id"),
            youtube_url: r.get("youtube_url"),
            video_title: r.get("video_title"),
            audio_path: r.get("audio_path"),
            video_path: r.get("video_path"),
            status: JobStatus::from_str(&status_str).unwrap_or(JobStatus::Error),
            error_message: r.get("error_message"),
            created_at: r.get("created_at"),
        }
    }))
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
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE mining_jobs SET audio_path = ?, video_title = ?, video_path = ? WHERE id = ?",
    )
    .bind(audio_path)
    .bind(video_title)
    .bind(video_path)
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
        let pool = test_pool().await;
        sqlx::raw_sql(MIGRATION).execute(&pool).await.unwrap();
    }

    #[tokio::test]
    async fn create_and_get_job() {
        let pool = test_pool().await;

        let id = create_job(&pool, "https://youtube.com/watch?v=abc").await.unwrap();
        let job = get_job(&pool, id).await.unwrap().unwrap();

        assert_eq!(job.youtube_url, "https://youtube.com/watch?v=abc");
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
    async fn update_job_status_sets_status_and_error() {
        let pool = test_pool().await;
        let id = create_job(&pool, "https://youtube.com/watch?v=abc").await.unwrap();

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
        let id = create_job(&pool, "https://youtube.com/watch?v=abc").await.unwrap();

        update_job_download(&pool, id, "/tmp/audio.wav", "Test Video", "/tmp/video.mp4")
            .await
            .unwrap();
        let job = get_job(&pool, id).await.unwrap().unwrap();
        assert_eq!(job.audio_path.as_deref(), Some("/tmp/audio.wav"));
        assert_eq!(job.video_title.as_deref(), Some("Test Video"));
    }

    #[tokio::test]
    async fn insert_and_get_sentences() {
        let pool = test_pool().await;
        let job_id = create_job(&pool, "https://youtube.com/watch?v=abc").await.unwrap();

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
        let job_id = create_job(&pool, "https://youtube.com/watch?v=abc").await.unwrap();

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
}
