//! `lookups` — every Yomitan dictionary popup, as observed by the proxy.
//!
//! Written by [`crate::ankiproxy`], which sits between Yomitan and AnkiConnect
//! and counts the duplicate checks Yomitan fires while *displaying* a
//! definition. That makes "I didn't know this word" an observable event without
//! Yomitan cooperating in any way.
//!
//! Two uses, and they want different things from the same rows: as *presence
//! marks* (proof the reader was at the keyboard) only the timestamps matter,
//! and as the mining funnel the term matters. `spec/knowledge-db.md` generalizes
//! this into the ledger's `encounters` log, tagged by source type.

use sqlx::{Row, SqlitePool};

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
