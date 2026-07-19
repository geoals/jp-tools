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

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use tracing::{debug, warn};

use crate::app::AppState;
use crate::db;

/// Actions Yomitan issues while *displaying* a definition. Anything else
/// (adding notes, media, version probes) is forwarded without counting.
const LOOKUP_ACTIONS: &[&str] = &["findNotes", "canAddNotes", "canAddNotesWithErrorDetail"];

/// Window over which repeated requests for the same term collapse into one
/// lookup. Covers the burst a single popup emits without merging a genuine
/// re-lookup of the same word later in the same sentence.
const DEDUPE_SECS: f64 = 3.0;

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// CORS headers for the browser-extension origin Yomitan calls from. Upstream
/// AnkiConnect's own CORS headers are deliberately not forwarded: it allows
/// only the origins in its `webCorsOriginList`, and this proxy is reachable
/// only on the local network anyway.
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

/// Pull the looked-up term out of an AnkiConnect request body.
///
/// Yomitan expresses the duplicate check two ways depending on version and
/// settings — a search query (`findNotes`) or full candidate notes
/// (`canAddNotes`) — so both shapes are tried. `vocab_field` is the note field
/// holding the dictionary form (JP_TOOLS_ANKI_FIELD_VOCAB).
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

/// Read the value of `<field>:` out of an Anki search query. Anki escapes `"`
/// and `*` in the value with a backslash; unescaping keeps the recorded term
/// equal to the word as it appears on the card.
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
        }
        // Not our business to reject what Anki might accept — forward it.
        Err(e) => debug!(error = %e, "unparseable proxy body, forwarding as-is"),
    }

    forward(&state, body).await
}

async fn record(state: &AppState, term: &str) {
    let work = match db::load_settings(&state.pool).await {
        Ok(s) => s.current_work,
        Err(e) => {
            warn!(error = %e, "lookup work lookup failed, recording without work");
            String::new()
        }
    };
    let work = (!work.is_empty()).then_some(work);

    match db::insert_lookup(&state.pool, now_ts(), term, work.as_deref(), DEDUPE_SECS).await {
        Ok(true) => debug!(term, "lookup recorded"),
        Ok(false) => debug!(term, "lookup deduped"),
        // Counting lookups must never break mining: log and forward anyway.
        Err(e) => warn!(error = %e, term, "failed to record lookup"),
    }
}

async fn forward(state: &AppState, body: Bytes) -> Response {
    let resp = state
        .http
        .post(&state.anki_url)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, url = %state.anki_url, "AnkiConnect unreachable");
            return (StatusCode::BAD_GATEWAY, cors_headers(), e.to_string()).into_response();
        }
    };

    let status = resp.status();
    match resp.bytes().await {
        Ok(bytes) => {
            let mut headers = cors_headers();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            (status, headers, bytes).into_response()
        }
        Err(e) => {
            warn!(error = %e, "AnkiConnect response unreadable");
            (StatusCode::BAD_GATEWAY, cors_headers(), e.to_string()).into_response()
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
