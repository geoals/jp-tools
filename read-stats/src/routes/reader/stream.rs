//! The live line feed, as server-sent events.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_core::Stream;
use serde::Deserialize;
use tracing::warn;

use super::highlight;
use crate::app::AppState;
use crate::db;

/// How often the stream checks for new lines — the whole of the pipeline's
/// controllable latency, since the logger commits the moment Textractor hooks a
/// line. A poll of N ms costs a uniform 0..N delay, and at 250ms the feed read
/// as perceptibly behind the voice. 30ms costs ~33 queries/sec per reader, each
/// an index seek past the end of `lines`; WAL readers don't block the writer.
const POLL_INTERVAL: Duration = Duration::from_millis(30);

/// Cap on a single catch-up batch, so a client resuming after hours away
/// doesn't pull the whole history in one go. Also the ceiling on the opening
/// session (see below), for the same reason.
const MAX_BATCH: i64 = 500;

/// Floor on the opening batch, so a feed does not open on a handful of lines.
///
/// The view wants to open on the sitting in progress, but a sitting that has
/// just started is a few lines. Below this the batch reaches past the session
/// boundary into what came before, which the feed already groups and headers.
/// 200 is one backscroll page (`HISTORY_PAGE` in reader.js).
const MIN_OPENING_LINES: i64 = 200;

#[derive(Deserialize)]
pub struct StreamQuery {
    /// Resume after this line id instead of sending a backlog.
    pub after: Option<i64>,
    /// Fixed number of trailing lines to open with, instead of the whole
    /// current sitting. Only a caller that wants a *short* feed has any use
    /// for this; the reading view deliberately does not pass it.
    pub backlog: Option<i64>,
}

/// Server-sent events, one per hooked line, `data` being the line JSON.
///
/// Each event carries its line id, so a browser that drops the connection
/// (screen off, tab backgrounded) reconnects with `Last-Event-ID` and
/// resumes exactly where it left off rather than replaying the backlog.
pub async fn lines_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<StreamQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let resume = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok())
        .or(q.after);
    let backlog = q.backlog.map(|n| n.clamp(0, MAX_BATCH));

    let stream = async_stream::stream! {
        // Built here rather than per line: the first caller pays the dictionary
        // load, everyone after it pays nothing. `None` streams untinted.
        let hl = highlight::shared(&state).await;

        // Opening batch: everything missed since `resume`, or — for a fresh
        // client — the whole sitting in progress, so the view opens with the
        // session it is part of rather than an arbitrary tail of it, widened to
        // `MIN_OPENING_LINES` when that sitting is a short one. Anything older
        // is a scroll back, served by `/api/lines/before`.
        let mut last_id = match resume {
            Some(id) => id,
            None => match opening_batch(&state, backlog).await {
                Ok(lines) => {
                    let last = lines.last().map(|l| l.id);
                    for line in &lines {
                        yield Ok(line_event(line, tokens(&state, hl.as_deref(), line).await));
                    }
                    match last {
                        Some(id) => id,
                        // Empty table: start from the current end so the first
                        // hooked line still arrives.
                        None => db::max_line_id(&state.knowledge).await.unwrap_or(0),
                    }
                }
                Err(e) => {
                    warn!(error = %e, "reader backlog failed");
                    0
                }
            },
        };

        loop {
            match db::fetch_lines_after_id(&state.knowledge, last_id, MAX_BATCH).await {
                Ok(lines) => {
                    for line in &lines {
                        last_id = line.id;
                        yield Ok(line_event(line, tokens(&state, hl.as_deref(), line).await));
                    }
                }
                // A transient DB error must not end the stream — the client
                // would reconnect into the same error anyway.
                Err(e) => warn!(error = %e, "reader poll failed"),
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    };

    // Comment pings keep the connection open through idle timeouts.
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// What a fresh client opens with: the whole sitting in progress — widened to
/// `MIN_OPENING_LINES` if that sitting is short — or a fixed tail if the caller
/// asked for one.
async fn opening_batch(
    state: &AppState,
    backlog: Option<i64>,
) -> Result<Vec<db::ReaderLine>, sqlx::Error> {
    let Some(n) = backlog else {
        let gap = match db::load_settings(&state.local).await {
            Ok(s) => s.session_gap_secs,
            // The feed matters more than the grouping: open on a plain tail
            // rather than dropping the reader on a blank page.
            Err(e) => {
                warn!(error = %e, "reader backlog: settings unreadable, using default gap");
                600.0
            }
        };
        let session = db::fetch_current_session_lines(&state.knowledge, gap, MAX_BATCH).await?;
        // A short sitting opens with the reading that preceded it rather than
        // with itself alone. `fetch_recent_lines` is a superset of what the
        // session query just returned — both take the newest lines, one of them
        // stopping at the gap — so this replaces that answer rather than adding
        // to it, and cannot double a line.
        if (session.len() as i64) < MIN_OPENING_LINES {
            return db::fetch_recent_lines(&state.knowledge, MIN_OPENING_LINES).await;
        }
        return Ok(session);
    };
    db::fetch_recent_lines(&state.knowledge, n).await
}

/// A line as the reading view receives it: the row, plus where its unknown
/// words sit.
///
/// Flattened over [`db::ReaderLine`] rather than nested: `tokens` is an
/// addition to the event, not a new shape for it.
#[derive(serde::Serialize)]
struct LineEvent<'a> {
    #[serde(flatten)]
    line: &'a db::ReaderLine,
    /// Empty whenever the pipeline could not answer. The line still arrives,
    /// just untinted.
    tokens: Vec<highlight::Span>,
}

/// One line's spans, or none if there is no highlighter to ask.
async fn tokens(
    state: &AppState,
    hl: Option<&highlight::Highlighter>,
    line: &db::ReaderLine,
) -> Vec<highlight::Span> {
    match hl {
        Some(h) => highlight::spans(&state.knowledge, h, &line.text).await,
        None => Vec::new(),
    }
}

fn line_event(line: &db::ReaderLine, tokens: Vec<highlight::Span>) -> Event {
    // json_data only fails on non-serializable values; both halves are plain data.
    Event::default()
        .id(line.id.to_string())
        .json_data(LineEvent { line, tokens })
        .unwrap_or_else(|_| Event::default().comment("unserializable line"))
}
