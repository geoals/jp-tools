//! What a work contains, from its script, before it has been read.
//!
//! [`super::work_terms`] answers "which words have I met in this work" and
//! fills line by line as reading happens. This answers "which words are in it
//! at all", in one pass over the extracted script, and so can be asked about a
//! work not started — which is the only way to know whether the next one in the
//! queue is at the edge of what can be read or a year away.
//!
//! Same key as the ledger and the same pipeline behind it, so the join to
//! `vocabulary` is exact. Both figures the per-work page draws apply here too:
//! by type is how much studying a work needs, by token is how it will feel.
//!
//! Coverage is over the whole script, so for a branching work it counts every
//! route. That makes it a lower bound on any one playthrough.

use sqlx::Row;

use super::Knowledge;
use super::vocabulary::Status;
use super::work_terms::{IS_KNOWN, WorkEncounter, WorkTerm};

/// How many rows go in one transaction. A profile is tens of thousands of rows
/// and SQLite takes one write lock per database, so a single transaction would
/// hold it for the whole import — long enough for the texthooker appending to
/// `lines` beside it to time out. Short transactions give it gaps to write in.
const CHUNK: usize = 2_000;

/// Replace one work's profile with a freshly derived one.
///
/// A whole-work replace rather than the additive write `work_terms` takes: a
/// script is imported complete, and re-importing it is a re-derivation of the
/// same text, not more of it.
///
/// The `work_scripts` row is written last, after every term row, so it doubles
/// as the completion marker: an import interrupted part way leaves no profile
/// rather than a profile that silently under-counts.
pub async fn record_script(
    k: &Knowledge,
    work: &str,
    total_terms: i64,
    batch: &[WorkEncounter],
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM work_scripts WHERE work = ?")
        .bind(work)
        .execute(k.pool())
        .await?;
    sqlx::query("DELETE FROM work_script_terms WHERE work = ?")
        .bind(work)
        .execute(k.pool())
        .await?;

    for chunk in batch.chunks(CHUNK) {
        let mut tx = k.pool().begin().await?;
        for e in chunk {
            sqlx::query(
                "INSERT INTO work_script_terms (work, headword, reading, count) \
                 VALUES (?, ?, ?, ?) \
                 ON CONFLICT(work, headword, reading) DO UPDATE SET count = excluded.count",
            )
            .bind(work)
            .bind(&e.term.headword)
            .bind(&e.term.reading)
            .bind(e.count)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
    }

    sqlx::query(
        "INSERT INTO work_scripts (work, total_terms, parsed_at) \
         VALUES (?, ?, datetime('now')) \
         ON CONFLICT(work) DO UPDATE SET \
             total_terms = excluded.total_terms, parsed_at = excluded.parsed_at",
    )
    .bind(work)
    .bind(total_terms)
    .execute(k.pool())
    .await?;
    Ok(())
}

/// How much of a work's script is already known, by type and by token.
#[derive(Debug, Default, Clone)]
pub struct ScriptCoverage {
    pub types: i64,
    pub tokens: i64,
    pub known_types: i64,
    pub known_tokens: i64,
    pub unknown_types: i64,
    pub new_types: i64,
}

impl ScriptCoverage {
    /// The share of running text already known — what reading it will feel
    /// like, as against how much studying it would take.
    pub fn token_coverage(&self) -> f64 {
        if self.tokens == 0 {
            return 0.0;
        }
        self.known_tokens as f64 / self.tokens as f64
    }
}

/// A ledger row exists only for a word met while reading, so a script's
/// never-encountered words have none — and they are the whole point of asking.
/// Joined the other way (the inner join `work_terms` can afford, its terms
/// having been met by definition) they vanish, and the work reads as easier
/// than it is by exactly the words that make it hard.
///
/// So: left join, and no row means not known. Wordhood was already settled at
/// tokenizing time by `counts_as_word`; the ledger's stricter dictionary gate
/// only applies where there is a row to apply it to.
const IS_WORD_OR_UNMET: &str = "(v.headword IS NULL OR (v.in_master = 1 OR v.in_name = 1 \
     OR v.in_reference = 1))";

pub async fn coverage(k: &Knowledge, work: &str) -> Result<ScriptCoverage, sqlx::Error> {
    let known = format!("(v.headword IS NOT NULL AND {IS_KNOWN})");
    let row = sqlx::query(&format!(
        "SELECT \
             COUNT(*) AS types, \
             COALESCE(SUM(ws.count), 0) AS tokens, \
             COALESCE(SUM({known}), 0) AS known_types, \
             COALESCE(SUM(CASE WHEN {known} THEN ws.count ELSE 0 END), 0) AS known_tokens, \
             COALESCE(SUM(v.status = '{unknown}'), 0) AS unknown_types, \
             COALESCE(SUM(v.headword IS NULL OR v.status = '{new}'), 0) AS new_types \
         FROM work_script_terms ws LEFT JOIN vocabulary v \
             ON v.headword = ws.headword AND v.reading = ws.reading \
         WHERE ws.work = ? AND {IS_WORD_OR_UNMET}",
        unknown = Status::Unknown.as_str(),
        new = Status::New.as_str(),
    ))
    .bind(work)
    .fetch_one(k.pool())
    .await?;

    Ok(ScriptCoverage {
        types: row.get("types"),
        tokens: row.get("tokens"),
        known_types: row.get("known_types"),
        known_tokens: row.get("known_tokens"),
        unknown_types: row.get("unknown_types"),
        new_types: row.get("new_types"),
    })
}

/// How many of the script's distinct words have actually been met in it.
///
/// Progress through a work's *vocabulary*, which is not progress through its
/// text: the words met early are the ones it repeats, so this trails the
/// character count and the gap between them is the work's long tail.
pub async fn met_types(k: &Knowledge, work: &str) -> Result<i64, sqlx::Error> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS met FROM work_script_terms ws \
         WHERE ws.work = ? AND EXISTS ( \
             SELECT 1 FROM work_terms wt \
             WHERE wt.work = ws.work AND wt.headword = ws.headword \
               AND wt.reading = ws.reading)",
    )
    .bind(work)
    .fetch_one(k.pool())
    .await?;
    Ok(row.get("met"))
}

/// One term waiting to be judged, with what the work says about it.
#[derive(Debug, Clone)]
pub struct TriageTerm {
    pub headword: String,
    pub reading: String,
    /// Occurrences in this work's script — the order the session runs in.
    pub count: i64,
    /// Rank in the reader-facing frequency list, for "common in Japanese
    /// generally" as against "common here". None where it is unranked.
    pub rank: Option<i64>,
    /// Times met in reading so far. Zero means this word is being judged on
    /// sight, which is why nothing here may be preselected.
    pub met: i64,
}

/// The script's unjudged words, commonest in this work first.
///
/// Never-judged only: a word already marked stays marked, and a `known` verdict
/// under another reading of the same headword takes the whole headword out, the
/// same rule triage and the per-work lists use.
///
/// Rows with no ledger entry at all are the point of this queue rather than an
/// edge case — they are the words the reader has never met, so they can never
/// reach the encounter-driven triage queue, and they are most of what a script
/// holds that reading has not yet covered.
pub async fn triage_queue(
    k: &Knowledge,
    work: &str,
    frequency_dictionary: i64,
    limit: i64,
) -> Result<Vec<TriageTerm>, sqlx::Error> {
    let rows = sqlx::query(&format!(
        "SELECT ws.headword, ws.reading, ws.count, \
                COALESCE(v.encounter_count, 0) AS met, \
                (SELECT MIN(f.frequency) FROM dictionary_frequency f \
                  WHERE f.dictionary_id = ? AND f.term = ws.headword) AS rank \
         FROM work_script_terms ws LEFT JOIN vocabulary v \
             ON v.headword = ws.headword AND v.reading = ws.reading \
         WHERE ws.work = ? \
           AND (v.headword IS NULL OR v.status = '{new}') \
           AND COALESCE(v.mined, 0) = 0 \
           AND NOT EXISTS (SELECT 1 FROM vocabulary o \
                            WHERE o.headword = ws.headword AND o.status = 'known') \
         ORDER BY ws.count DESC, ws.headword LIMIT ?",
        new = Status::New.as_str(),
    ))
    .bind(frequency_dictionary)
    .bind(work)
    .bind(limit)
    .fetch_all(k.pool())
    .await?;
    Ok(rows
        .iter()
        .map(|r| TriageTerm {
            headword: r.get("headword"),
            reading: r.get("reading"),
            count: r.get("count"),
            rank: r.get("rank"),
            met: r.get("met"),
        })
        .collect())
}

/// The unknown words the script leans on hardest — ranked by how often they
/// occur *in this work*, which is what makes it a list worth learning before
/// starting rather than a generic frequency deck.
pub async fn top_unknown(
    k: &Knowledge,
    work: &str,
    limit: i64,
) -> Result<Vec<WorkTerm>, sqlx::Error> {
    let rows = sqlx::query(&format!(
        "SELECT ws.headword, ws.reading, v.pos, \
                COALESCE(v.status, '{new}') AS status, ws.count, \
                COALESCE(v.encounter_count, 0) AS elsewhere \
         FROM work_script_terms ws LEFT JOIN vocabulary v \
             ON v.headword = ws.headword AND v.reading = ws.reading \
         WHERE ws.work = ? AND {IS_WORD_OR_UNMET} \
           AND NOT (v.headword IS NOT NULL AND {IS_KNOWN}) \
           AND (v.headword IS NULL OR v.status IN ('{new}', '{unknown}')) \
         ORDER BY ws.count DESC, ws.headword LIMIT ?",
        new = Status::New.as_str(),
        unknown = Status::Unknown.as_str(),
    ))
    .bind(work)
    .bind(limit)
    .fetch_all(k.pool())
    .await?;
    Ok(rows
        .iter()
        .map(|r| WorkTerm {
            headword: r.get("headword"),
            reading: r.get("reading"),
            pos: r.get("pos"),
            status: r.get("status"),
            count: r.get("count"),
            // Encounters anywhere in reading so far: a script word already met
            // is a different proposition from one never seen.
            elsewhere: r.get("elsewhere"),
        })
        .collect())
}
