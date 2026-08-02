//! How each term was written where it was met.
//!
//! The ledger's key is Sudachi's normalized form, so a row saying 窺う may have
//! been read as うかがう every single time. This sink keeps what the page
//! actually said, and one line id per spelling to prove it.
//!
//! Counts are per *surface*, inflection included — 出来る and 出来れ are two
//! rows. Splitting them costs nothing and folding them would need the same
//! normalization that lost the information in the first place.

use sqlx::Row;

use super::Knowledge;
use super::vocabulary::Term;

/// One term written one way, from a batch of newly ingested text.
#[derive(Debug, Clone)]
pub struct SurfaceEncounter {
    pub term: Term,
    pub surface: String,
    pub count: i64,
    /// The line it was met in. `None` for manually logged session text, which
    /// has no per-line ids.
    pub line_id: Option<i64>,
}

/// One spelling of a term, as stored.
#[derive(Debug, Clone)]
pub struct SurfaceRow {
    pub surface: String,
    pub count: i64,
    pub line_id: Option<i64>,
}

/// Add a batch of per-surface counts.
///
/// Additive and not idempotent, like the other ingest sinks: the caller is
/// watermarked. `line_id` is kept from the first batch that supplies one, so
/// re-running a rebuild lands on the same example line.
pub async fn record_surfaces(
    k: &Knowledge,
    batch: &[SurfaceEncounter],
) -> Result<(), sqlx::Error> {
    if batch.is_empty() {
        return Ok(());
    }
    let mut tx = k.pool().begin().await?;
    for e in batch {
        sqlx::query(
            "INSERT INTO term_surfaces (headword, reading, surface, count, line_id) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(headword, reading, surface) DO UPDATE SET \
                 count = count + excluded.count, \
                 line_id = COALESCE(term_surfaces.line_id, excluded.line_id)",
        )
        .bind(&e.term.headword)
        .bind(&e.term.reading)
        .bind(&e.surface)
        .bind(e.count)
        .bind(e.line_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// Every spelling one term was met in, commonest first.
pub async fn for_term(k: &Knowledge, term: &Term) -> Result<Vec<SurfaceRow>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT surface, count, line_id FROM term_surfaces \
         WHERE headword = ? AND reading = ? ORDER BY count DESC, surface",
    )
    .bind(&term.headword)
    .bind(&term.reading)
    .fetch_all(k.pool())
    .await?;
    Ok(rows
        .iter()
        .map(|r| SurfaceRow {
            surface: r.get("surface"),
            count: r.get("count"),
            line_id: r.get("line_id"),
        })
        .collect())
}

/// Clear the sink, for the rebuild path. The watermarks are rewound beside it.
pub async fn reset(k: &Knowledge) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM term_surfaces")
        .execute(k.pool())
        .await?;
    Ok(())
}
