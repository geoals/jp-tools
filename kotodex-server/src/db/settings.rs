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
    /// Minutes a day needs to extend the streak. Separate from the target: the
    /// streak asks "did you show up", the target asks "did you do the work".
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
    /// How many times a word must have been met before triage offers it, and
    /// defaults it to `known`. It can sit this low because the default is only
    /// reached by words never looked up — see
    /// `jp_core::knowledge::vocabulary::preselects_known`.
    pub triage_min_encounters: i64,
    /// Highest rank the reading view calls *common*. A word at or above
    /// this rank that is `new` or `unknown` is underlined: not knowing a rare
    /// word is expected, not knowing a common one is the gap worth seeing.
    pub reader_common_max_freq_rank: i64,
    /// The same threshold against BCCWJ, tested independently: the two corpora
    /// disagree about which words are common, and a word common in newspaper
    /// and government prose is a gap worth seeing even when the fiction list
    /// ranks it rare. Underlined if either rank passes.
    pub reader_common_max_bccwj_rank: i64,
    /// Capture is suspended: vn-ws-logger.py closes its Textractor WebSocket
    /// while this is set, so nothing reaches the line stream at all. Stopping
    /// the source beats the old interval log, which left the raw stream full of
    /// text the reader had said was not reading.
    pub capture_paused: bool,
    /// Paint each word with what the ledger says about it. Off means the spans
    /// are still there — they are the click targets — and simply carry no
    /// status class. An empty ledger paints nothing either way, so a fresh
    /// install reads plain text without having to be told to.
    pub highlight_status: bool,
    /// Where lines come from: `ws` is Textractor through its WebSocket plugin,
    /// `clipboard` is whatever a clipboard hooker copies. One producer either
    /// way — `vn-ws-logger.py` switches source rather than a second writer
    /// existing, so the filters, the dedup and the ruby split stay one
    /// implementation.
    pub line_source: String,
    /// The WebSocket to hook. Textractor's plugin defaults to 6677, but it is
    /// configurable there and a second hooker uses another port.
    pub line_source_ws_url: String,
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
            reader_common_max_freq_rank: 5000,
            reader_common_max_bccwj_rank: 10000,
            capture_paused: false,
            highlight_status: true,
            line_source: "ws".into(),
            line_source_ws_url: "ws://localhost:6677".into(),
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
    "reader_common_max_freq_rank",
    "reader_common_max_bccwj_rank",
    "capture_paused",
    "highlight_status",
    "line_source",
    "line_source_ws_url",
];

/// Settings whose stored value is `"1"`/`"0"` rather than a number or free text.
pub const BOOL_SETTING_KEYS: &[&str] = &["capture_paused", "highlight_status"];

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
            "reader_common_max_freq_rank" => {
                settings.reader_common_max_freq_rank = value
                    .parse()
                    .unwrap_or(settings.reader_common_max_freq_rank)
            }
            "reader_common_max_bccwj_rank" => {
                settings.reader_common_max_bccwj_rank = value
                    .parse()
                    .unwrap_or(settings.reader_common_max_bccwj_rank)
            }
            "capture_paused" => settings.capture_paused = value == "1",
            "highlight_status" => settings.highlight_status = value == "1",
            "line_source" => {
                if !value.is_empty() {
                    settings.line_source = value
                }
            }
            "line_source_ws_url" => {
                if !value.is_empty() {
                    settings.line_source_ws_url = value
                }
            }
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
