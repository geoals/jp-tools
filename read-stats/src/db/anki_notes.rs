//! `anki_notes` — a mirror of the mined deck, never the source of truth.
//!
//! Anki owns "is this word mined". This table is a snapshot taken through
//! AnkiConnect and replaced wholesale on every refresh, so drift is fixed by
//! re-syncing rather than by reconciling. Nothing here is ever written back to
//! Anki.
//!
//! The note id doubles as the card's creation time in epoch **milliseconds**,
//! which is why mined-card timestamps can be derived without a second query —
//! and why ids are kept sorted, so a window count is a pair of binary searches.

use jp_core::knowledge::Knowledge;
use sqlx::Row;

#[derive(Debug, Clone)]
pub struct AnkiNote {
    /// Anki note id — also the note's creation time in epoch milliseconds.
    pub note_id: i64,
    /// The card's own spelling, as the text spelt it.
    pub vocab: String,
    /// The ledger key it stands for — [`crate::ingest::normalized_spellings`].
    /// Empty for a snapshot taken before the column existed; every reader falls
    /// back to `vocab` there, and the next refresh fills it.
    pub headword: String,
}

impl AnkiNote {
    /// What to join this card against anything the tokenizer produced —
    /// `word_days` lemmas, `vocabulary` rows. The fallback is what makes a
    /// pre-column snapshot behave as it did before rather than match nothing.
    pub fn key(&self) -> &str {
        if self.headword.is_empty() {
            &self.vocab
        } else {
            &self.headword
        }
    }
}

/// Replace the deck snapshot wholesale (it mirrors, never owns, the deck).
pub async fn replace_anki_notes(k: &Knowledge, notes: &[AnkiNote]) -> Result<(), sqlx::Error> {
    let mut tx = k.pool().begin().await?;
    sqlx::query("DELETE FROM anki_notes")
        .execute(&mut *tx)
        .await?;
    for n in notes {
        sqlx::query(
            "INSERT OR REPLACE INTO anki_notes (note_id, vocab, headword) VALUES (?, ?, ?)",
        )
        .bind(n.note_id)
        .bind(&n.vocab)
        .bind(&n.headword)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

pub async fn fetch_anki_notes(k: &Knowledge) -> Result<Vec<AnkiNote>, sqlx::Error> {
    let rows = sqlx::query("SELECT note_id, vocab, headword FROM anki_notes ORDER BY note_id")
        .fetch_all(k.pool())
        .await?;
    Ok(rows
        .iter()
        .map(|r| AnkiNote {
            note_id: r.get("note_id"),
            vocab: r.get("vocab"),
            headword: r.get("headword"),
        })
        .collect())
}

pub async fn fetch_anki_note_ids(k: &Knowledge) -> Result<Vec<i64>, sqlx::Error> {
    let rows = sqlx::query("SELECT note_id FROM anki_notes ORDER BY note_id")
        .fetch_all(k.pool())
        .await?;
    Ok(rows.iter().map(|r| r.get("note_id")).collect())
}
