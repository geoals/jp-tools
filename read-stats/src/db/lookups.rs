//! `lookups` — every Yomitan dictionary popup made *while reading*, as observed
//! by the proxy.
//!
//! Written by [`crate::ankiproxy`], which sits between Yomitan and AnkiConnect
//! and counts the duplicate checks Yomitan fires while *displaying* a
//! definition. That makes "I didn't know this word" an observable event without
//! Yomitan cooperating in any way.
//!
//! **Every row here is inside a reading session.** Yomitan is pointed at the
//! proxy from the browser, so it also fires for articles, tweets and forum
//! posts; `ankiproxy::record` drops those before they land, on the test that a
//! line arrived within `session_gap_secs`. Nothing downstream re-checks it, and
//! nothing downstream should have to — the guard is at the write, so a lookup
//! that exists is a lookup that counts. 76 rows written before the guard
//! existed (2026-07-26) were deleted rather than filtered, for the same reason:
//! a row that means "somewhere else" has no reading to belong to.
//!
//! Two uses wanting different things from the same rows: as *presence marks*
//! (proof the reader was at the keyboard) only the timestamps matter, and as the
//! mining funnel the term matters.

use jp_core::knowledge::Knowledge;
use sqlx::Row;

/// Record one Yomitan lookup, unless the same term was already recorded within
/// `dedupe_secs`. One popup display can fire several AnkiConnect requests (a
/// duplicate check per definition entry), and paging through a popup re-fires
/// them; collapsing by term over a short window makes one popup one lookup.
///
/// Returns whether a row was written.
pub async fn insert_lookup(
    k: &Knowledge,
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
    .execute(k.pool())
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Lookup timestamps in a window, oldest first.
pub async fn fetch_lookup_events(
    k: &Knowledge,
    from_ts: f64,
    to_ts: f64,
) -> Result<Vec<f64>, sqlx::Error> {
    let rows = sqlx::query("SELECT ts FROM lookups WHERE ts >= ? AND ts < ? ORDER BY ts")
        .bind(from_ts)
        .bind(to_ts)
        .fetch_all(k.pool())
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

pub async fn fetch_lookup_terms(k: &Knowledge) -> Result<Vec<LookupTerm>, sqlx::Error> {
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
    .fetch_all(k.pool())
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

/// Every spelling Yomitan sent that has not been resolved to a ledger key yet.
///
/// Distinct, because the normalization is per spelling and a term is looked up
/// many times.
pub async fn unnormalized_lookup_terms(k: &Knowledge) -> Result<Vec<String>, sqlx::Error> {
    let rows = sqlx::query("SELECT DISTINCT term FROM lookups WHERE term <> '' AND headword = ''")
        .fetch_all(k.pool())
        .await?;
    Ok(rows.iter().map(|r| r.get("term")).collect())
}

/// Store the ledger key for each spelling, one statement per distinct spelling.
pub async fn set_lookup_headwords(
    k: &Knowledge,
    resolved: &[(String, String)],
) -> Result<u64, sqlx::Error> {
    let mut tx = k.pool().begin().await?;
    let mut n = 0;
    for (term, headword) in resolved {
        n += sqlx::query("UPDATE lookups SET headword = ? WHERE term = ? AND headword = ''")
            .bind(headword)
            .bind(term)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    }
    tx.commit().await?;
    Ok(n)
}
