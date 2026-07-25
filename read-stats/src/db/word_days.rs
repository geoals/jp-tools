//! `word_days` — per-day counts of each content word in the line stream.
//!
//! Written incrementally by [`crate::ingest`] behind a watermark, so a refresh
//! only tokenizes lines it hasn't seen. Its one consumer is the mined-word
//! re-encounter panel: "of the words I carded, which ones has the reading
//! actually shown me again?"
//!
//! `spec/knowledge-db.md` drops this table once the `vocabulary` ledger exists
//! — it is derivable from `lines`, and only survives today because there is no
//! ledger to derive it into.

use sqlx::{Row, SqlitePool};

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
