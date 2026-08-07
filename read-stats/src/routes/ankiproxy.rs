//! AnkiConnect pass-through that counts Yomitan lookups.
//!
//! Yomitan checks Anki for duplicates every time it displays a definition
//! popup, so pointing its "Server address" at this endpoint turns every lookup
//! into an observable event. Requests are forwarded to the real AnkiConnect
//! byte-for-byte and the response returned unchanged, so mining behaves exactly
//! as it did before — this sits in the path but never alters it.
//!
//! Only *read* actions count. Adding a card is preceded by the popup that
//! already counted, so counting `addNote` too would double up.
//!
//! read-stats' own AnkiConnect client (anki.rs) talks to Anki directly rather
//! than through here, so a refresh can't inflate the lookup count.
//!
//! Adding a card is not this module's job. An `addNote` is handed to
//! [`crate::services::card::add_note`], the one function every card path calls,
//! and the reply is relayed unchanged like any other.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use tracing::{debug, warn};

use crate::app::AppState;
use crate::clock::now_ts;
use crate::db;
use crate::services::card;

/// Actions Yomitan issues while *displaying* a definition. Anything else
/// (adding notes, media, version probes) is forwarded without counting.
const LOOKUP_ACTIONS: &[&str] = &["findNotes", "canAddNotes", "canAddNotesWithErrorDetail"];

/// Window over which repeated requests for one term collapse into a single
/// lookup — long enough for a popup's burst, short enough not to merge a real
/// re-lookup later in the same sentence.
const DEDUPE_SECS: f64 = 3.0;

/// CORS headers for the browser-extension origin Yomitan calls from. Upstream
/// AnkiConnect's own are not forwarded — it allows only the origins in its
/// `webCorsOriginList`, and this proxy is local-network only.
fn cors_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("POST, OPTIONS"),
    );
    headers
}

pub async fn preflight() -> Response {
    (StatusCode::NO_CONTENT, cors_headers()).into_response()
}

/// Pull the looked-up term out of an AnkiConnect request body. Yomitan
/// expresses the duplicate check either as a search query (`findNotes`) or as
/// full candidate notes (`canAddNotes`), so both shapes are tried.
pub fn extract_term(body: &Value, vocab_field: &str) -> Option<String> {
    let params = body.get("params")?;

    // canAddNotes: {"notes": [{"fields": {"VocabKanji": "単語", ...}}, ...]}
    if let Some(notes) = params.get("notes").and_then(Value::as_array) {
        for note in notes {
            if let Some(term) = note
                .get("fields")
                .and_then(|f| f.get(vocab_field))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                return Some(term.to_string());
            }
        }
    }

    // findNotes: {"query": "\"VocabKanji:単語\""}, possibly with a deck or
    // note-type clause alongside it depending on Yomitan's duplicate scope.
    if let Some(query) = params.get("query").and_then(Value::as_str) {
        return term_from_query(query, vocab_field);
    }

    None
}

/// Read the value of `<field>:` out of an Anki search query. Anki backslash-
/// escapes `"` and `*`; unescaping keeps the recorded term equal to the word as
/// it appears on the card.
fn term_from_query(query: &str, field: &str) -> Option<String> {
    let start = query.find(&format!("{field}:"))? + field.len() + 1;
    let rest = &query[start..];

    let mut term = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => term.extend(chars.next()),
            '"' => break,
            _ => term.push(c),
        }
    }
    let term = term.trim();
    (!term.is_empty()).then(|| term.to_string())
}

pub async fn proxy(State(state): State<AppState>, body: Bytes) -> Response {
    // Record before forwarding: a lookup happened whether or not Anki is up.
    let mut is_add = false;
    match serde_json::from_slice::<Value>(&body) {
        Ok(parsed) => {
            let action = parsed.get("action").and_then(Value::as_str).unwrap_or("");
            if LOOKUP_ACTIONS.contains(&action) {
                if let Some(term) = extract_term(&parsed, &state.anki_vocab_field) {
                    record(&state, &term).await;
                } else {
                    debug!(action, "lookup action with no extractable term");
                }
            }
            is_add = action == "addNote";
        }
        // Not our business to reject what Anki might accept — forward it.
        Err(e) => debug!(error = %e, "unparseable proxy body, forwarding as-is"),
    }

    // Forward byte-for-byte and relay the response unchanged — the proxy's
    // contract. Enrichment, which `add_note` starts, is a *separate* follow-up
    // write, never a mutation of what Yomitan sent or what it gets back.
    let forwarded = if is_add {
        card::add_note(&state, body).await
    } else {
        card::forward(&state, body).await
    };
    let (status, resp_bytes) = match forwarded {
        Ok(pair) => pair,
        Err(e) => return (StatusCode::BAD_GATEWAY, cors_headers(), e).into_response(),
    };

    let mut headers = cors_headers();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    (status, headers, resp_bytes).into_response()
}

/// Record a lookup, but only one made while a VN was actually being read.
///
/// Yomitan is pointed here from the browser, so it fires for anything looked up
/// anywhere — an article, a tweet. Those are real lookups but not *this*
/// reading, and admitting them puts terms the VN never contained into the
/// per-work funnel and into the numerator of every rate whose denominator the
/// line stream cannot see.
///
/// The test is a line within `session_gap_secs`, the same threshold that ends a
/// session everywhere else. It follows that a pause outlasting that gap stops
/// lookups too, since no lines arrive while `capture_paused` is set.
/// Returns the id of the row this lookup is accounted to, for the one caller
/// that may have to take it back — see [`crate::routes::reader::define`].
pub(crate) async fn record(state: &AppState, term: &str) -> Option<i64> {
    let settings = match db::load_settings(&state.local).await {
        Ok(s) => s,
        Err(e) => {
            // Without settings there is no window to test against and no work
            // to stamp. Dropping the lookup is the safe half of the trade: a
            // missed lookup understates a rate, a stray one corrupts a work.
            warn!(error = %e, term, "settings unavailable, not recording lookup");
            return None;
        }
    };

    match db::line_within(&state.knowledge, now_ts(), settings.session_gap_secs).await {
        Ok(true) => {}
        Ok(false) => {
            debug!(term, "lookup outside a reading session, not recorded");
            return None;
        }
        // Counting lookups must never break mining, and it must not silently
        // drop them either: if the question cannot be asked, keep the row.
        Err(e) => warn!(error = %e, term, "session check failed, recording anyway"),
    }

    let work = (!settings.current_work.is_empty()).then_some(settings.current_work);

    match db::insert_lookup(
        &state.knowledge,
        now_ts(),
        term,
        work.as_deref(),
        DEDUPE_SECS,
    )
    .await
    {
        Ok(id) => {
            debug!(term, id = ?id, "lookup recorded");
            id
        }
        // Counting lookups must never break mining: log and forward anyway.
        Err(e) => {
            warn!(error = %e, term, "failed to record lookup");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_term_from_can_add_notes() {
        let body = json!({
            "action": "canAddNotes",
            "params": { "notes": [{ "fields": { "VocabKanji": "邂逅", "Sentence": "..." } }] }
        });
        assert_eq!(extract_term(&body, "VocabKanji").as_deref(), Some("邂逅"));
    }

    #[test]
    fn extracts_term_from_find_notes_query() {
        let body = json!({
            "action": "findNotes",
            "params": { "query": "\"VocabKanji:邂逅\"" }
        });
        assert_eq!(extract_term(&body, "VocabKanji").as_deref(), Some("邂逅"));
    }

    #[test]
    fn extracts_term_from_scoped_query() {
        let body = json!({
            "action": "findNotes",
            "params": { "query": "\"deck:Japanese\" \"VocabKanji:邂逅\"" }
        });
        assert_eq!(extract_term(&body, "VocabKanji").as_deref(), Some("邂逅"));
    }

    #[test]
    fn unescapes_query_values() {
        assert_eq!(
            term_from_query("\"VocabKanji:a\\\"b\"", "VocabKanji").as_deref(),
            Some("a\"b")
        );
    }

    #[test]
    fn ignores_requests_without_a_term() {
        let version = json!({ "action": "version", "params": {} });
        assert_eq!(extract_term(&version, "VocabKanji"), None);

        let notes_info = json!({ "action": "notesInfo", "params": { "notes": [1, 2] } });
        assert_eq!(extract_term(&notes_info, "VocabKanji"), None);

        let empty = json!({
            "action": "canAddNotes",
            "params": { "notes": [{ "fields": { "VocabKanji": "" } }] }
        });
        assert_eq!(extract_term(&empty, "VocabKanji"), None);
    }

    #[test]
    fn honours_a_renamed_vocab_field() {
        let body = json!({
            "action": "canAddNotes",
            "params": { "notes": [{ "fields": { "Word": "邂逅" } }] }
        });
        assert_eq!(extract_term(&body, "Word").as_deref(), Some("邂逅"));
        assert_eq!(extract_term(&body, "VocabKanji"), None);
    }
}
