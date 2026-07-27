//! `settings` — runtime-tunable thresholds, goals, and app state.
//!
//! One key/value row per setting, overlaid on the defaults below, so a value
//! that has never been set has exactly one definition (here) rather than one in
//! the schema and one in code. Keys outside [`SETTING_KEYS`] are internal
//! bookkeeping (snapshot timestamps, the ingest watermark) and are read with
//! [`get_setting_raw`] instead — the API refuses to write them.

use sqlx::{Row, SqlitePool};

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
    /// Daily reading target, in minutes — the one goal the meter draws.
    pub goal_target_mins: i64,
    /// Minutes a day needs before it extends the streak. Deliberately separate
    /// from the target: the streak asks "did you show up", the target asks
    /// "did you do the day's work", and one number cannot answer both without
    /// making a short day either free or worthless.
    pub streak_min_mins: i64,
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
    /// How many times a word must have been met before vocabulary triage
    /// offers it, and defaults it to `known`. The default is only reached by
    /// words never looked up, which is what lets it sit this low: an
    /// encounter count says "met often", a zero lookup count says "and never
    /// needed help with it", and the two together are the evidence. See
    /// `jp_core::knowledge::vocabulary::preselects_known`.
    pub triage_min_encounters: i64,
    /// Capture is suspended: vn-ws-logger.py closes its Textractor WebSocket
    /// and stays disconnected while this is set, so nothing reaches the line
    /// stream at all. This replaced the old `pauses` interval log — excluding
    /// spans after the fact meant the raw stream still filled up with text the
    /// reader had explicitly said wasn't reading.
    pub capture_paused: bool,
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
            goal_target_mins: 120,
            streak_min_mins: 60,
            chars_per_page: 550.0,
            current_work: String::new(),
            pace_start_date: String::new(),
            vn_window: String::new(),
            // 3, because the lookup-count half of the rule carries most of the
            // weight: met three times and never once looked up is already a
            // meaningful signal, and a higher floor mostly just shortens the
            // queue. Tune it from the settings page against a real queue.
            triage_min_encounters: 3,
            capture_paused: false,
        }
    }
}

pub const SETTING_KEYS: &[&str] = &[
    "afk_secs",
    "session_gap_secs",
    "day_rollover_hour",
    "goal_target_mins",
    "streak_min_mins",
    "chars_per_page",
    "current_work",
    "pace_start_date",
    "vn_window",
    "triage_min_encounters",
    "capture_paused",
];

/// Settings whose stored value is `"1"`/`"0"` rather than a number or free text.
pub const BOOL_SETTING_KEYS: &[&str] = &["capture_paused"];

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
            "goal_target_mins" => {
                settings.goal_target_mins = value.parse().unwrap_or(settings.goal_target_mins)
            }
            "streak_min_mins" => {
                settings.streak_min_mins = value.parse().unwrap_or(settings.streak_min_mins)
            }
            "chars_per_page" => {
                settings.chars_per_page = value.parse().unwrap_or(settings.chars_per_page)
            }
            "current_work" => settings.current_work = value,
            "pace_start_date" => settings.pace_start_date = value,
            "vn_window" => settings.vn_window = value,
            "triage_min_encounters" => {
                settings.triage_min_encounters =
                    value.parse().unwrap_or(settings.triage_min_encounters)
            }
            "capture_paused" => settings.capture_paused = value == "1",
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

/// Read one settings row that isn't part of the user-facing Settings struct
/// (snapshot timestamps, ingest watermark).
pub async fn get_setting_raw(pool: &SqlitePool, key: &str) -> Result<Option<String>, sqlx::Error> {
    Ok(sqlx::query("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?
        .map(|r| r.get("value")))
}
