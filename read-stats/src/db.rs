use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

use crate::stats::LineEvent;

const MIGRATION: &str = include_str!("../migrations/001_create_stats_tables.sql");
const MIGRATION_WORKS: &str = include_str!("../migrations/002_create_works.sql");
const MIGRATION_ANKI: &str = include_str!("../migrations/003_create_anki_tables.sql");
const MIGRATION_LOOKUPS: &str = include_str!("../migrations/004_create_lookups.sql");
const MIGRATION_LOOKUP_IDX: &str = include_str!("../migrations/005_create_lookup_indexes.sql");

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

    // ALTER TABLE ADD COLUMN has no IF NOT EXISTS in SQLite — DBs created
    // before the work column need it added.
    if !has_column(&pool, "lines", "work").await? {
        sqlx::raw_sql("ALTER TABLE lines ADD COLUMN work TEXT")
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
    /// Days before this ISO date are excluded from the finish-date pace
    /// window (set after a reading break so old zero days don't drag the
    /// estimate). Empty = no cutoff.
    pub pace_start_date: String,
    /// Substring of the VN window's title, passed to vn-capture.sh as
    /// VN_WINDOW so it screenshots the VN by id rather than whatever has
    /// focus. Empty = capture the focused window (the old behaviour).
    pub vn_window: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            // 30s ≈ p90 of measured lookup gaps (median 24s, tight cluster to
            // ~32s): keeps a genuine lookup whole while truncating the tail
            // where a lookup turned into a distraction.
            afk_secs: 30.0,
            session_gap_secs: 600.0,
            day_rollover_hour: 4,
            goal_floor_mins: 60,
            goal_target_mins: 120,
            chars_per_page: 550.0,
            current_work: String::new(),
            pace_start_date: String::new(),
            vn_window: String::new(),
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
    "pace_start_date",
    "vn_window",
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
            "pace_start_date" => settings.pace_start_date = value,
            "vn_window" => settings.vn_window = value,
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

/// A hooked line as the reader view shows it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReaderLine {
    pub id: i64,
    pub ts: f64,
    pub chars: i64,
    pub text: String,
}

/// Lines newer than `after_id`, oldest first. The reader's SSE loop calls this
/// on a short interval, so it stays a bounded index range scan.
pub async fn fetch_lines_after_id(
    pool: &SqlitePool,
    after_id: i64,
    limit: i64,
) -> Result<Vec<ReaderLine>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, ts, chars, text FROM lines
         WHERE id > ? AND text IS NOT NULL ORDER BY id LIMIT ?",
    )
    .bind(after_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(reader_line).collect())
}

/// The newest `limit` lines, oldest first — the backlog a reader gets on open
/// so the screen isn't blank until the next line is hooked.
pub async fn fetch_recent_lines(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<ReaderLine>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, ts, chars, text FROM lines
         WHERE text IS NOT NULL ORDER BY id DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    let mut lines: Vec<ReaderLine> = rows.iter().map(reader_line).collect();
    lines.reverse();
    Ok(lines)
}

fn reader_line(r: &sqlx::sqlite::SqliteRow) -> ReaderLine {
    ReaderLine {
        id: r.get("id"),
        ts: r.get("ts"),
        chars: r.get("chars"),
        text: r.get::<Option<String>, _>("text").unwrap_or_default(),
    }
}

/// Highest line id currently stored, or 0 when the table is empty.
pub async fn max_line_id(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    let row = sqlx::query("SELECT COALESCE(MAX(id), 0) AS max_id FROM lines")
        .fetch_one(pool)
        .await?;
    Ok(row.get("max_id"))
}

pub async fn fetch_line_events(
    pool: &SqlitePool,
    from_ts: f64,
    to_ts: f64,
) -> Result<Vec<LineEvent>, sqlx::Error> {
    let rows = sqlx::query("SELECT ts, chars, text FROM lines WHERE ts >= ? AND ts < ? ORDER BY ts")
        .bind(from_ts)
        .bind(to_ts)
        .fetch_all(pool)
        .await?;

    // One scanner across the whole stream: a speech broken over several text
    // boxes leaves its 「 open on the first row, so depth has to carry. It is
    // dropped across a break too long for that to be what happened — see
    // `dialogue::CARRY_GAP_SECS`.
    let mut scanner = crate::dialogue::Scanner::new();
    let mut prev_ts: Option<f64> = None;
    Ok(rows
        .iter()
        .map(|r| {
            let ts: f64 = r.get("ts");
            let chars: i64 = r.get("chars");
            let text: Option<String> = r.get("text");
            if prev_ts.is_some_and(|p| ts - p > crate::dialogue::CARRY_GAP_SECS) {
                scanner.reset();
            }
            prev_ts = Some(ts);
            match text {
                Some(text) => {
                    let split = scanner.scan(&text);
                    LineEvent {
                        ts,
                        chars,
                        // `chars` is authoritative (startup recounts it), so
                        // clamp rather than let a stale disagreement make
                        // narration negative.
                        dialogue_chars: split.dialogue.min(chars),
                        classified: true,
                    }
                }
                None => {
                    scanner.reset();
                    LineEvent { ts, chars, dialogue_chars: 0, classified: false }
                }
            }
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
    Ok(
        sqlx::query("SELECT id FROM pauses WHERE end_ts IS NULL LIMIT 1")
            .fetch_optional(pool)
            .await?
            .is_some(),
    )
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

pub const WORK_STATUSES: &[&str] = &["reading", "queued", "finished", "dropped"];

#[derive(Debug, serde::Serialize)]
pub struct Work {
    pub id: i64,
    pub title: String,
    pub total_chars: Option<i64>,
    pub cover_path: Option<String>,
    pub status: String,
    pub queue_pos: Option<i64>,
}

const WORK_COLS: &str = "id, title, total_chars, cover_path, status, queue_pos";

fn work_from_row(r: &sqlx::sqlite::SqliteRow) -> Work {
    Work {
        id: r.get("id"),
        title: r.get("title"),
        total_chars: r.get("total_chars"),
        cover_path: r.get("cover_path"),
        status: r.get("status"),
        queue_pos: r.get("queue_pos"),
    }
}

pub async fn fetch_works_meta(pool: &SqlitePool) -> Result<Vec<Work>, sqlx::Error> {
    let rows = sqlx::query(&format!("SELECT {WORK_COLS} FROM works ORDER BY id"))
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(work_from_row).collect())
}

pub async fn fetch_work(pool: &SqlitePool, id: i64) -> Result<Option<Work>, sqlx::Error> {
    let row = sqlx::query(&format!("SELECT {WORK_COLS} FROM works WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.as_ref().map(work_from_row))
}

/// Get-or-create a work row by its exact title (the lines/sessions join key).
pub async fn upsert_work(pool: &SqlitePool, title: &str) -> Result<Work, sqlx::Error> {
    sqlx::query("INSERT INTO works (title) VALUES (?) ON CONFLICT(title) DO NOTHING")
        .bind(title)
        .execute(pool)
        .await?;
    let row = sqlx::query(&format!("SELECT {WORK_COLS} FROM works WHERE title = ?"))
        .bind(title)
        .fetch_one(pool)
        .await?;
    Ok(work_from_row(&row))
}

pub async fn set_work_cover(
    pool: &SqlitePool,
    id: i64,
    cover_path: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE works SET cover_path = ? WHERE id = ?")
        .bind(cover_path)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_work_total_chars(
    pool: &SqlitePool,
    id: i64,
    total_chars: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE works SET total_chars = ? WHERE id = ?")
        .bind(total_chars)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_work_status(pool: &SqlitePool, id: i64, status: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE works SET status = ? WHERE id = ?")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_work_queue_pos(
    pool: &SqlitePool,
    id: i64,
    queue_pos: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE works SET queue_pos = ? WHERE id = ?")
        .bind(queue_pos)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_work(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM works WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Read one settings row that isn't part of the user-facing Settings struct
/// (snapshot timestamps, ingest watermark).
pub async fn get_setting_raw(pool: &SqlitePool, key: &str) -> Result<Option<String>, sqlx::Error> {
    Ok(sqlx::query("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?
        .map(|r| r.get("value")))
}

#[derive(Debug, Clone)]
pub struct AnkiNote {
    /// Anki note id — also the note's creation time in epoch milliseconds.
    pub note_id: i64,
    pub vocab: String,
}

/// Replace the deck snapshot wholesale (it mirrors, never owns, the deck).
pub async fn replace_anki_notes(pool: &SqlitePool, notes: &[AnkiNote]) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM anki_notes").execute(&mut *tx).await?;
    for n in notes {
        sqlx::query("INSERT OR REPLACE INTO anki_notes (note_id, vocab) VALUES (?, ?)")
            .bind(n.note_id)
            .bind(&n.vocab)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

pub async fn fetch_anki_notes(pool: &SqlitePool) -> Result<Vec<AnkiNote>, sqlx::Error> {
    let rows = sqlx::query("SELECT note_id, vocab FROM anki_notes ORDER BY note_id")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| AnkiNote {
            note_id: r.get("note_id"),
            vocab: r.get("vocab"),
        })
        .collect())
}

pub async fn fetch_anki_note_ids(pool: &SqlitePool) -> Result<Vec<i64>, sqlx::Error> {
    let rows = sqlx::query("SELECT note_id FROM anki_notes ORDER BY note_id")
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| r.get("note_id")).collect())
}

#[derive(Debug)]
pub struct IngestLine {
    pub id: i64,
    pub ts: f64,
    pub text: String,
}

pub async fn fetch_lines_after(
    pool: &SqlitePool,
    after_id: i64,
) -> Result<Vec<IngestLine>, sqlx::Error> {
    let rows =
        sqlx::query("SELECT id, ts, text FROM lines WHERE id > ? AND text IS NOT NULL ORDER BY id")
            .bind(after_id)
            .fetch_all(pool)
            .await?;
    Ok(rows
        .iter()
        .map(|r| IngestLine {
            id: r.get("id"),
            ts: r.get("ts"),
            text: r.get("text"),
        })
        .collect())
}

pub async fn add_word_day_counts(
    pool: &SqlitePool,
    counts: &[(String, String, i64)], // (lemma, date, count)
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for (lemma, date, count) in counts {
        sqlx::query(
            "INSERT INTO word_days (lemma, date, count) VALUES (?, ?, ?)
             ON CONFLICT(lemma, date) DO UPDATE SET count = count + excluded.count",
        )
        .bind(lemma)
        .bind(date)
        .bind(count)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// Record one Yomitan lookup, unless the same term was already recorded within
/// `dedupe_secs`. One popup display can fire several AnkiConnect requests (a
/// duplicate check per definition entry), and paging through a popup re-fires
/// them; collapsing by term over a short window makes one popup one lookup.
///
/// Returns whether a row was written.
pub async fn insert_lookup(
    pool: &SqlitePool,
    ts: f64,
    term: &str,
    work: Option<&str>,
    dedupe_secs: f64,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO lookups (ts, term, work)
         SELECT ?, ?, ?
         WHERE NOT EXISTS (
             SELECT 1 FROM lookups WHERE term = ? AND ts > ?
         )",
    )
    .bind(ts)
    .bind(term)
    .bind(work)
    .bind(term)
    .bind(ts - dedupe_secs)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Lookup timestamps in a window, oldest first.
pub async fn fetch_lookup_events(
    pool: &SqlitePool,
    from_ts: f64,
    to_ts: f64,
) -> Result<Vec<f64>, sqlx::Error> {
    let rows = sqlx::query("SELECT ts FROM lookups WHERE ts >= ? AND ts < ? ORDER BY ts")
        .bind(from_ts)
        .bind(to_ts)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| r.get("ts")).collect())
}

/// One distinct looked-up term, with the earliest mined card carrying it (if
/// any). `note_id` is epoch milliseconds, so comparing it against `first_ts`
/// tells mined-because-of-this-lookup apart from already-had-a-card.
#[derive(Debug)]
pub struct LookupTerm {
    pub term: String,
    pub times: i64,
    pub first_ts: f64,
    pub last_ts: f64,
    pub note_id: Option<i64>,
    /// Latest lookup at or before the card's creation — the one that actually
    /// led to mining. Measuring from `first_ts` instead would report days for a
    /// word looked up long before it was finally carded.
    pub mine_from_ts: Option<f64>,
}

pub async fn fetch_lookup_terms(pool: &SqlitePool) -> Result<Vec<LookupTerm>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT t.term, t.times, t.first_ts, t.last_ts,
                (SELECT MIN(a.note_id) FROM anki_notes a WHERE a.vocab = t.term) AS note_id,
                (SELECT MAX(l.ts) FROM lookups l
                 WHERE l.term = t.term
                   AND l.ts <= (SELECT MIN(a.note_id) FROM anki_notes a WHERE a.vocab = t.term) / 1000.0
                ) AS mine_from_ts
         FROM (
             SELECT term, COUNT(*) AS times, MIN(ts) AS first_ts, MAX(ts) AS last_ts
             FROM lookups
             WHERE term IS NOT NULL AND term <> ''
             GROUP BY term
         ) t",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| LookupTerm {
            term: r.get("term"),
            times: r.get("times"),
            first_ts: r.get("first_ts"),
            last_ts: r.get("last_ts"),
            note_id: r.get("note_id"),
            mine_from_ts: r.get("mine_from_ts"),
        })
        .collect())
}

#[derive(Debug)]
pub struct WordDayHit {
    pub lemma: String,
    pub date: String,
    pub count: i64,
}

/// All word-day rows whose lemma matches a mined vocab entry.
pub async fn fetch_mined_word_days(pool: &SqlitePool) -> Result<Vec<WordDayHit>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT w.lemma, w.date, w.count FROM word_days w
         WHERE EXISTS (SELECT 1 FROM anki_notes a WHERE a.vocab = w.lemma)
         ORDER BY w.date",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| WordDayHit {
            lemma: r.get("lemma"),
            date: r.get("date"),
            count: r.get("count"),
        })
        .collect())
}
