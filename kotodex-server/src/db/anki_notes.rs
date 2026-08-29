//! `anki_notes` — a mirror of the mined deck, never the source of truth.
//!
//! Anki owns "is this word mined". This table is a snapshot taken through
//! AnkiConnect and replaced wholesale on every refresh, so drift is fixed by
//! re-syncing rather than by reconciling. Nothing here is ever written back to
//! Anki.
//!
//! One card at a time also lands here, from `services::card`, the moment Anki accepts
//! an `addNote` — see [`insert_anki_note`]. That does not make the table a
//! source of truth: the next refresh still replaces it wholesale.
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

/// Add the one card Anki has just accepted.
///
/// The refresh is the only writer that *owns* this table, and it still replaces
/// it wholesale — this only spares the mirror from being blind until the next
/// one. It had to be: the refresh runs when the dashboard page opens, mining
/// happens in the overlay for hours without that, and every cards-per-hour
/// figure reads this table. Cards mined in a session simply were not there.
///
/// One row, not a refresh: a refresh refetches the whole deck and resolves
/// every spelling through Sudachi, which must not sit on the path that adds a
/// card.
pub async fn insert_anki_note(k: &Knowledge, note: &AnkiNote) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT OR REPLACE INTO anki_notes (note_id, vocab, headword) VALUES (?, ?, ?)")
        .bind(note.note_id)
        .bind(&note.vocab)
        .bind(&note.headword)
        .execute(k.pool())
        .await
        .map(|_| ())
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

/// Whether any card has ever been mined into the mirror.
///
/// What tells a surface that mining is part of this install: Anki is optional,
/// and a reader who does not mine must not be told their Anki is down.
pub async fn any_anki_note(k: &Knowledge) -> Result<bool, sqlx::Error> {
    let row = sqlx::query("SELECT EXISTS(SELECT 1 FROM anki_notes) AS present")
        .fetch_one(k.pool())
        .await?;
    Ok(row.get::<i64, _>("present") != 0)
}
