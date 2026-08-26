//! Paper books — logging a book read on paper against its epub.
//!
//! The library owns the work; this module owns the epub underneath it. The
//! shelf adds a book (`upload`, `setup`); the book's own page logs sittings
//! against it (`preview`, `log`).
//!
//! Four calls, in the order they are made: **upload** the epub, **setup** to
//! say where the story starts and what pages the paper copy runs between,
//! then **preview** and **log** once per sitting.
//!
//! **skip** is the fifth and is used once, on a book already part-read when
//! its epub is added: it moves the position without writing a session.
//!
//! Preview and log are separate because the anchor search can land in the
//! wrong place — the same ten characters occur twice, or the reader typed a
//! line they had already logged — and the only reliable check is seeing the
//! text around it. Preview returns that context and the offset it found; log
//! takes the offset back rather than searching again, so what is saved is what
//! was confirmed.
//!
//! A logged sitting is an ordinary `manual_sessions` row carrying the text
//! between the two positions. Nothing downstream knows it came from paper:
//! ingest tokenizes the content into the ledger, `word_days` and the kanji
//! grid on its own watermark, and the character count is `count_chars` like
//! every other.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::books;
use crate::clock::now_ts;
use crate::db;
use crate::error::AppError;
use crate::routes::reader::highlight;

/// How much text to hand Sudachi at once when counting what a span is made of.
/// Split on line boundaries, so no word is cut in half by the chunking.
const ANALYZE_CHUNK_BYTES: usize = 8000;

pub async fn list_books(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let works = db::fetch_works_meta(&state.knowledge).await?;
    let books = db::fetch_books(&state.knowledge).await?;
    let out: Vec<Value> = books
        .into_iter()
        .map(|b| {
            let status = works
                .iter()
                .find(|w| w.title == b.work)
                .map(|w| w.status.clone());
            let mut v = serde_json::to_value(&b).unwrap();
            v["status"] = json!(status);
            v["chars_per_page"] = json!(books::chars_per_page(
                b.body_chars,
                b.first_page,
                b.last_page
            ));
            // Bytes, not characters: the whole point is a progress bar, and
            // counting the characters left would mean loading the text.
            let body_bytes = (b.text_bytes - b.body_start).max(1);
            v["progress"] = json!((b.position - b.body_start) as f64 / body_bytes as f64);
            v
        })
        .collect();
    Ok(Json(json!({ "books": out })))
}

#[derive(Deserialize)]
pub struct UploadParams {
    /// The exact title the sessions and the `works` row will carry.
    title: String,
}

/// Take an epub, flatten it, and create the book and its `works` row.
///
/// The body is the file itself rather than a multipart form: the client has
/// the bytes and there is exactly one field.
pub async fn upload_book(
    State(state): State<AppState>,
    Query(params): Query<UploadParams>,
    body: Bytes,
) -> Result<Json<Value>, AppError> {
    let title = params.title.trim();
    if title.is_empty() {
        return Err(AppError::BadRequest("a title is required".into()));
    }
    if db::fetch_book(&state.knowledge, title).await?.is_some() {
        return Err(AppError::BadRequest(format!(
            "{title} already has an epub — every position recorded is an offset into it"
        )));
    }
    let text = jp_core::epub::flatten(&body)
        .map_err(|e| AppError::BadRequest(format!("could not read the epub: {e}")))?;

    let work = db::upsert_work(&state.knowledge, title).await?;
    let book = db::insert_book(&state.knowledge, title, &text, now_ts()).await?;
    save_cover(&state, &work, &body).await;
    Ok(Json(json!({
        "book": book,
        // The opening of the file, so the start anchor can be typed off it
        // when the front matter is short enough to see.
        "head": text.chars().take(600).collect::<String>(),
    })))
}

/// Put the epub's own cover art on the shelf.
///
/// Best effort and never fatal: a book whose epub carries no findable cover is
/// still a book. An existing cover is left alone — a VNDB one was chosen and a
/// re-uploaded epub must not overwrite it.
async fn save_cover(state: &AppState, work: &db::Work, epub: &[u8]) {
    if work.cover_path.is_some() {
        return;
    }
    let Some(cover) = jp_core::epub::cover(epub) else {
        return;
    };
    let filename = format!("w{}.{}", work.id, cover.ext);
    let write = async {
        tokio::fs::create_dir_all(&state.covers_dir).await?;
        tokio::fs::write(state.covers_dir.join(&filename), &cover.bytes).await
    };
    if let Err(e) = write.await {
        tracing::warn!(error = %e, "could not store the epub cover");
        return;
    }
    if let Err(e) = db::set_work_cover(&state.knowledge, work.id, Some(&filename)).await {
        tracing::warn!(error = %e, "could not record the epub cover");
    }
}

#[derive(Deserialize)]
pub struct SetupBody {
    pub work: String,
    /// A few characters from the first line of the story.
    pub anchor: String,
    /// The printed page numbers the body text runs between.
    pub first_page: Option<i64>,
    pub last_page: Option<i64>,
}

pub async fn setup_book(
    State(state): State<AppState>,
    Json(req): Json<SetupBody>,
) -> Result<Json<Value>, AppError> {
    let text = db::fetch_book_text(&state.knowledge, &req.work)
        .await?
        .ok_or(AppError::NotFound)?;
    // From 0: setup is the one search that may look at the front matter,
    // because it is what puts the position past it.
    let found = books::find(&text, 0, req.anchor.trim())
        .ok_or_else(|| AppError::BadRequest("that text is not in the epub".into()))?;

    let body_chars = jp_core::text::chars::count_chars(&text[found.start..]);
    db::books::set_setup(
        &state.knowledge,
        &req.work,
        found.start as i64,
        body_chars,
        req.first_page,
        req.last_page,
    )
    .await?;
    Ok(Json(json!({
        "found": found,
        "body_chars": body_chars,
        "chars_per_page": books::chars_per_page(body_chars, req.first_page, req.last_page),
    })))
}

#[derive(Deserialize)]
pub struct PreviewBody {
    pub work: String,
    pub anchor: String,
    /// Where to search from. Absent means the stored position; set to a past
    /// match's `start` + 1 to reject it and keep looking.
    pub from: Option<i64>,
    pub minutes: Option<f64>,
}

pub async fn preview_book(
    State(state): State<AppState>,
    Json(req): Json<PreviewBody>,
) -> Result<Json<Value>, AppError> {
    let book = db::fetch_book(&state.knowledge, &req.work)
        .await?
        .ok_or(AppError::NotFound)?;
    let text = db::fetch_book_text(&state.knowledge, &req.work)
        .await?
        .ok_or(AppError::NotFound)?;

    let from = req.from.unwrap_or(book.position).max(book.position) as usize;
    let found = books::find(&text, from, req.anchor.trim()).ok_or_else(|| {
        AppError::BadRequest(
            "that text is not in the rest of the book — check it against the page, or type more of it".into(),
        )
    })?;

    let span = &text[book.position as usize..found.end];
    let chars = jp_core::text::chars::count_chars(span);
    let pages = books::chars_per_page(book.body_chars, book.first_page, book.last_page)
        .map(|cpp| chars as f64 / cpp);

    Ok(Json(json!({
        "found": found,
        "chars": chars,
        "pages": pages,
        "speed": req.minutes.filter(|m| *m > 0.0).map(|m| (chars as f64 / m) * 60.0),
        "words": words_in(&state, span).await,
    })))
}

/// What the span is made of, by ledger status — unique terms, not tokens.
///
/// The same pipeline the reading view tints with, so a word counted `new` here
/// is a word `#read` would have marked. It is a read of how hard the stretch
/// was; nothing is written, and ingest does the real ledger pass later on its
/// own watermark.
async fn words_in(state: &AppState, span: &str) -> Value {
    let Some(h) = highlight::shared(state).await else {
        return Value::Null;
    };
    let mut counts: std::collections::HashMap<&'static str, std::collections::HashSet<String>> =
        std::collections::HashMap::new();
    for chunk in chunks(span, ANALYZE_CHUNK_BYTES) {
        for t in highlight::analyze(&state.knowledge, &h, chunk).await {
            let Some(status) = t.status else { continue };
            counts
                .entry(status)
                .or_default()
                .insert(format!("{}\u{0}{}", t.headword, t.reading));
        }
    }
    let n = |k: &str| counts.get(k).map_or(0, |s| s.len());
    json!({
        "new": n("new"),
        "seen": n("seen"),
        "unknown": n("unknown"),
        "known": n("known"),
    })
}

/// Split on line boundaries into pieces of at most `max` bytes. A single line
/// longer than that is yielded whole rather than cut — the tokenizer would
/// rather have one long line than a word in two halves.
fn chunks(text: &str, max: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut end = 0;
    for line in text.split_inclusive('\n') {
        let next = end + line.len();
        if next - start > max && end > start {
            out.push(&text[start..end]);
            start = end;
        }
        end = next;
    }
    if end > start {
        out.push(&text[start..end]);
    }
    out
}

#[derive(Deserialize)]
pub struct SkipBody {
    pub work: String,
    /// The offset a preview confirmed.
    pub end: i64,
}

/// Move the position without logging anything.
///
/// For a book already part-read when its epub is added: those pages were read
/// before there was anything to record them, and inventing a session for them
/// would credit a day that did not happen and push the characters through the
/// ledger as if they had just been met.
pub async fn skip_book(
    State(state): State<AppState>,
    Json(req): Json<SkipBody>,
) -> Result<Json<Value>, AppError> {
    let book = position_target(&state, &req.work, req.end).await?;
    db::books::set_position(&state.knowledge, &req.work, req.end).await?;
    Ok(Json(json!({ "position": req.end, "was": book.position })))
}

/// Check an offset is a place in this book that is actually ahead.
async fn position_target(state: &AppState, work: &str, end: i64) -> Result<db::Book, AppError> {
    let book = db::fetch_book(&state.knowledge, work)
        .await?
        .ok_or(AppError::NotFound)?;
    if end <= book.position {
        return Err(AppError::BadRequest(
            "that is at or behind where you already are".into(),
        ));
    }
    let text = db::fetch_book_text(&state.knowledge, work)
        .await?
        .ok_or(AppError::NotFound)?;
    if end as usize > text.len() || !text.is_char_boundary(end as usize) {
        return Err(AppError::BadRequest("not a position in this book".into()));
    }
    Ok(book)
}

#[derive(Deserialize)]
pub struct LogBody {
    pub work: String,
    /// The offset a preview confirmed. Not searched for again: what is saved
    /// has to be what was looked at.
    pub end: i64,
    pub minutes: Option<f64>,
    pub date: Option<String>,
}

pub async fn log_book(
    State(state): State<AppState>,
    Json(req): Json<LogBody>,
) -> Result<Json<Value>, AppError> {
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    if req.minutes.is_some_and(|m| !(m > 0.0)) {
        return Err(AppError::BadRequest("minutes must be > 0".into()));
    }
    let book = position_target(&state, &req.work, req.end).await?;
    let text = db::fetch_book_text(&state.knowledge, &req.work)
        .await?
        .ok_or(AppError::NotFound)?;
    let end = req.end as usize;
    let span = &text[book.position as usize..end];

    let settings = db::load_settings(&state.local).await?;
    let start_ts = crate::routes::sessions::resolve_start_ts(
        None,
        req.date.as_deref(),
        req.minutes,
        &settings,
    )?;
    let pages = books::chars_per_page(book.body_chars, book.first_page, book.last_page)
        .map(|cpp| jp_core::text::chars::count_chars(span) as f64 / cpp);

    let session = db::insert_session(
        &state.knowledge,
        db::NewSession {
            start_ts,
            end_ts: req.minutes.map(|m| start_ts + m * 60.0),
            chars: jp_core::text::chars::count_chars(span),
            source: "book",
            work: Some(&req.work),
            pages,
            content: Some(span),
            ..Default::default()
        },
    )
    .await?;
    db::books::set_position(&state.knowledge, &req.work, req.end).await?;
    Ok(Json(json!({ "session": session, "position": req.end })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_break_on_lines_and_cover_the_text() {
        let text = "あいう\nかきく\nさしす\n";
        let got = chunks(text, 10);
        assert_eq!(got.concat(), text);
        assert!(got.len() > 1);
        assert!(got.iter().all(|c| c.ends_with('\n')));
    }

    #[test]
    fn a_line_longer_than_the_chunk_is_not_cut() {
        let text = "あいうえおかきくけこ\n";
        assert_eq!(chunks(text, 5), vec![text]);
    }
}
