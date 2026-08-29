//! `books` — the flattened epub of a book being read on paper, and how far
//! through it the reader is.
//!
//! knowledge.db. One row per work, keyed on the same exact title string
//! `lines.work` and `manual_sessions.work` carry.
//!
//! A whole book's text dwarfs everything else on the row and the list view never
//! needs it, so [`Book`] does not carry it and [`fetch_text`] is a separate read.

use jp_core::knowledge::Knowledge;
use sqlx::Row;

#[derive(Debug, serde::Serialize)]
pub struct Book {
    pub work: String,
    /// Byte offsets into the flattened text. See the migration.
    pub body_start: i64,
    pub position: i64,
    pub body_chars: i64,
    pub text_bytes: i64,
    pub first_page: Option<i64>,
    pub last_page: Option<i64>,
    pub added_ts: f64,
}

const COLUMNS: &str =
    "work, body_start, position, body_chars, text_bytes, first_page, last_page, added_ts";

fn from_row(r: &sqlx::sqlite::SqliteRow) -> Book {
    Book {
        work: r.get("work"),
        body_start: r.get("body_start"),
        position: r.get("position"),
        body_chars: r.get("body_chars"),
        text_bytes: r.get("text_bytes"),
        first_page: r.get("first_page"),
        last_page: r.get("last_page"),
        added_ts: r.get("added_ts"),
    }
}

pub async fn fetch_books(k: &Knowledge) -> Result<Vec<Book>, sqlx::Error> {
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM books ORDER BY added_ts DESC"
    ))
    .fetch_all(k.pool())
    .await?;
    Ok(rows.iter().map(from_row).collect())
}

pub async fn fetch_book(k: &Knowledge, work: &str) -> Result<Option<Book>, sqlx::Error> {
    let row = sqlx::query(&format!("SELECT {COLUMNS} FROM books WHERE work = ?"))
        .bind(work)
        .fetch_optional(k.pool())
        .await?;
    Ok(row.as_ref().map(from_row))
}

pub async fn fetch_text(k: &Knowledge, work: &str) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query("SELECT text FROM books WHERE work = ?")
        .bind(work)
        .fetch_optional(k.pool())
        .await?;
    Ok(row.map(|r| r.get("text")))
}

/// Add a book. Fails when one is already stored for this work: every position
/// already recorded is an offset into *this* text, and a second flattening
/// would move all of them.
pub async fn insert_book(
    k: &Knowledge,
    work: &str,
    text: &str,
    added_ts: f64,
) -> Result<Book, sqlx::Error> {
    let row = sqlx::query(&format!(
        "INSERT INTO books (work, text, text_bytes, body_chars, added_ts) \
         VALUES (?, ?, ?, ?, ?) RETURNING {COLUMNS}",
    ))
    .bind(work)
    .bind(text)
    .bind(text.len() as i64)
    .bind(jp_core::text::chars::count_chars(text))
    .bind(added_ts)
    .fetch_one(k.pool())
    .await?;
    Ok(from_row(&row))
}

/// Where the body text starts and what the paper copy pages it between.
/// Resets `position` to the new start: a book being set up has not been read.
pub async fn set_setup(
    k: &Knowledge,
    work: &str,
    body_start: i64,
    body_chars: i64,
    first_page: Option<i64>,
    last_page: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE books SET body_start = ?, body_chars = ?, position = ?, \
         first_page = ?, last_page = ? WHERE work = ?",
    )
    .bind(body_start)
    .bind(body_chars)
    .bind(body_start)
    .bind(first_page)
    .bind(last_page)
    .bind(work)
    .execute(k.pool())
    .await?;
    Ok(())
}

pub async fn set_position(k: &Knowledge, work: &str, position: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE books SET position = ? WHERE work = ?")
        .bind(position)
        .bind(work)
        .execute(k.pool())
        .await?;
    Ok(())
}
