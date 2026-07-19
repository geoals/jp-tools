use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

use crate::stats::LineEvent;

const MIGRATION: &str = include_str!("../migrations/001_create_stats_tables.sql");

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

    // ALTER TABLE ADD COLUMN has no IF NOT EXISTS in SQLite — DBs created
    // before the work column need it added.
    if !has_column(&pool, "lines", "work").await? {
        sqlx::raw_sql("ALTER TABLE lines ADD COLUMN work TEXT")
            .execute(&pool)
            .await?;
    }
    Ok(pool)
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

/// Runtime-tunable thresholds and goals, stored as rows in `settings` and
/// overlaid on these defaults.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Settings {
    /// Max seconds one inter-line gap can credit as reading time.
    pub afk_secs: f64,
    /// A gap above this closes the session.
    pub session_gap_secs: f64,
    /// Hour at which a calendar day starts (late-night reading counts back).
    pub day_rollover_hour: i64,
    pub goal_floor_mins: i64,
    pub goal_target_mins: i64,
    /// Estimated characters per physical page (bunkobon default).
    pub chars_per_page: f64,
    /// Title stamped onto incoming hooked lines (set from the dashboard).
    pub current_work: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            afk_secs: 20.0,
            session_gap_secs: 600.0,
            day_rollover_hour: 4,
            goal_floor_mins: 60,
            goal_target_mins: 120,
            chars_per_page: 550.0,
            current_work: String::new(),
        }
    }
}

pub const SETTING_KEYS: &[&str] = &[
    "afk_secs",
    "session_gap_secs",
    "day_rollover_hour",
    "goal_floor_mins",
    "goal_target_mins",
    "chars_per_page",
    "current_work",
];

pub async fn load_settings(pool: &SqlitePool) -> Result<Settings, sqlx::Error> {
    let mut settings = Settings::default();
    let rows = sqlx::query("SELECT key, value FROM settings")
        .fetch_all(pool)
        .await?;
    for row in rows {
        let key: String = row.get("key");
        let value: String = row.get("value");
        match key.as_str() {
            "afk_secs" => settings.afk_secs = value.parse().unwrap_or(settings.afk_secs),
            "session_gap_secs" => {
                settings.session_gap_secs = value.parse().unwrap_or(settings.session_gap_secs)
            }
            "day_rollover_hour" => {
                settings.day_rollover_hour = value.parse().unwrap_or(settings.day_rollover_hour)
            }
            "goal_floor_mins" => {
                settings.goal_floor_mins = value.parse().unwrap_or(settings.goal_floor_mins)
            }
            "goal_target_mins" => {
                settings.goal_target_mins = value.parse().unwrap_or(settings.goal_target_mins)
            }
            "chars_per_page" => {
                settings.chars_per_page = value.parse().unwrap_or(settings.chars_per_page)
            }
            "current_work" => settings.current_work = value,
            _ => {}
        }
    }
    Ok(settings)
}

pub async fn save_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn fetch_line_events(
    pool: &SqlitePool,
    from_ts: f64,
    to_ts: f64,
) -> Result<Vec<LineEvent>, sqlx::Error> {
    let rows = sqlx::query("SELECT ts, chars FROM lines WHERE ts >= ? AND ts < ? ORDER BY ts")
        .bind(from_ts)
        .bind(to_ts)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| LineEvent {
            ts: r.get("ts"),
            chars: r.get("chars"),
        })
        .collect())
}

pub async fn fetch_work_lines(pool: &SqlitePool) -> Result<Vec<crate::stats::WorkLine>, sqlx::Error> {
    let rows = sqlx::query("SELECT ts, chars, work FROM lines ORDER BY ts")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| crate::stats::WorkLine {
            ts: r.get("ts"),
            chars: r.get("chars"),
            work: r.get("work"),
        })
        .collect())
}

pub async fn fetch_pauses(pool: &SqlitePool) -> Result<Vec<crate::stats::PauseInterval>, sqlx::Error> {
    let rows = sqlx::query("SELECT start_ts, end_ts FROM pauses ORDER BY start_ts")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| crate::stats::PauseInterval {
            start_ts: r.get("start_ts"),
            end_ts: r.get("end_ts"),
        })
        .collect())
}

/// Toggle the tracking pause; returns the new paused state.
pub async fn toggle_pause(pool: &SqlitePool, now: f64) -> Result<bool, sqlx::Error> {
    let open: Option<i64> = sqlx::query("SELECT id FROM pauses WHERE end_ts IS NULL LIMIT 1")
        .fetch_optional(pool)
        .await?
        .map(|r| r.get("id"));
    match open {
        Some(id) => {
            sqlx::query("UPDATE pauses SET end_ts = ? WHERE id = ?")
                .bind(now)
                .bind(id)
                .execute(pool)
                .await?;
            Ok(false)
        }
        None => {
            sqlx::query("INSERT INTO pauses (start_ts) VALUES (?)")
                .bind(now)
                .execute(pool)
                .await?;
            Ok(true)
        }
    }
}

pub async fn is_pause_open(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query("SELECT id FROM pauses WHERE end_ts IS NULL LIMIT 1")
        .fetch_optional(pool)
        .await?
        .is_some())
}

#[derive(Debug, serde::Serialize)]
pub struct ManualSession {
    pub id: i64,
    pub start_ts: f64,
    pub end_ts: f64,
    pub chars: i64,
    pub source: String,
    pub work: Option<String>,
    pub pages: Option<f64>,
    pub note: Option<String>,
}

fn manual_session_from_row(r: &sqlx::sqlite::SqliteRow) -> ManualSession {
    ManualSession {
        id: r.get("id"),
        start_ts: r.get("start_ts"),
        end_ts: r.get("end_ts"),
        chars: r.get("chars"),
        source: r.get("source"),
        work: r.get("work"),
        pages: r.get("pages"),
        note: r.get("note"),
    }
}

pub async fn fetch_sessions(
    pool: &SqlitePool,
    from_ts: f64,
    to_ts: f64,
) -> Result<Vec<ManualSession>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, start_ts, end_ts, chars, source, work, pages, note FROM sessions WHERE start_ts >= ? AND start_ts < ? ORDER BY start_ts",
    )
    .bind(from_ts)
    .bind(to_ts)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(manual_session_from_row).collect())
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_session(
    pool: &SqlitePool,
    start_ts: f64,
    end_ts: f64,
    chars: i64,
    source: &str,
    work: Option<&str>,
    pages: Option<f64>,
    note: Option<&str>,
) -> Result<ManualSession, sqlx::Error> {
    let row = sqlx::query(
        "INSERT INTO sessions (start_ts, end_ts, chars, source, work, pages, note) VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id, start_ts, end_ts, chars, source, work, pages, note",
    )
    .bind(start_ts)
    .bind(end_ts)
    .bind(chars)
    .bind(source)
    .bind(work)
    .bind(pages)
    .bind(note)
    .fetch_one(pool)
    .await?;
    Ok(manual_session_from_row(&row))
}

pub async fn delete_session(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM sessions WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
