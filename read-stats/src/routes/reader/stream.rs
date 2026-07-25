//! The live line feed, as server-sent events.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_core::Stream;
use serde::Deserialize;
use tracing::warn;

use crate::app::AppState;
use crate::db;

/// How often the stream checks for new lines, which is the whole of the
/// pipeline's controllable latency: vn-ws-logger.py commits in autocommit mode
/// the moment Textractor hooks a line, and the LAN hop is sub-millisecond. A
/// poll of N ms therefore costs a uniform 0..N delay — 250ms measured a mean of
/// 108ms, which reads as perceptibly behind the voice.
///
/// 30ms puts the mean at ~15ms, below the threshold where the line looks like
/// it lags the VN. The cost is ~33 queries/sec per connected reader, each an
/// index seek past the end of `lines` returning nothing — WAL readers don't
/// block the logger's writes, so this is not a contention risk either.
const POLL_INTERVAL: Duration = Duration::from_millis(30);

/// Lines shown on open when the client isn't resuming.
const DEFAULT_BACKLOG: i64 = 40;

/// Cap on a single catch-up batch, so a client resuming after hours away
/// doesn't pull the whole history in one go.
const MAX_BATCH: i64 = 500;

#[derive(Deserialize)]
pub struct StreamQuery {
    /// Resume after this line id instead of sending a backlog.
    pub after: Option<i64>,
    pub backlog: Option<i64>,
}

/// Server-sent events, one per hooked line, `data` being the line JSON.
///
/// Each event carries its line id, so a browser that drops the connection
/// (phone screen off, tab backgrounded) reconnects with `Last-Event-ID` and
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
    let backlog = q.backlog.unwrap_or(DEFAULT_BACKLOG).clamp(0, MAX_BATCH);

    let stream = async_stream::stream! {
        // Opening batch: everything missed since `resume`, or the tail of the
        // log for a fresh client.
        let mut last_id = match resume {
            Some(id) => id,
            None => match db::fetch_recent_lines(&state.knowledge, backlog).await {
                Ok(lines) => {
                    let last = lines.last().map(|l| l.id);
                    for line in &lines {
                        yield Ok(line_event(line));
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
                        yield Ok(line_event(line));
                    }
                }
                // A transient DB error must not end the stream — the client
                // would reconnect into the same error anyway.
                Err(e) => warn!(error = %e, "reader poll failed"),
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    };

    // Comment pings keep the connection open through the phone's idle timeouts.
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

fn line_event(line: &db::ReaderLine) -> Event {
    // json_data only fails on non-serializable values; ReaderLine is plain data.
    Event::default()
        .id(line.id.to_string())
        .json_data(line)
        .unwrap_or_else(|_| Event::default().comment("unserializable line"))
}
