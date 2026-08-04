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
use tracing::{debug, info, warn};

use crate::app::AppState;
use crate::db;

/// Actions Yomitan issues while *displaying* a definition. Anything else
/// (adding notes, media, version probes) is forwarded without counting.
const LOOKUP_ACTIONS: &[&str] = &["findNotes", "canAddNotes", "canAddNotesWithErrorDetail"];

/// Window over which repeated requests for one term collapse into a single
/// lookup — long enough for a popup's burst, short enough not to merge a real
/// re-lookup later in the same sentence.
const DEDUPE_SECS: f64 = 3.0;

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

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
    // Also remember an addNote, with the moment it arrived, so its *response*
    // (the new note id) can trigger media + CompactDef enrichment once Anki has
    // accepted it. That moment is the anchor, and it is taken here rather than
    // in the enrichment because this is the only point that still answers
    // "which line was on screen when the card was added" — everything after it,
    // Anki's own round-trip included, is time the reader can spend reading on.
    let mut added_note: Option<(Value, f64)> = None;
    match serde_json::from_slice::<Value>(&body) {
        Ok(parsed) => {
            let action = parsed.get("action").and_then(Value::as_str).unwrap_or("");
            if LOOKUP_ACTIONS.contains(&action) {
                if let Some(term) = extract_term(&parsed, &state.anki_vocab_field) {
                    record(&state, &term).await;
                } else {
                    debug!(action, "lookup action with no extractable term");
                }
            } else if action == "addNote" {
                added_note = Some((parsed, now_ts()));
            }
        }
        // Not our business to reject what Anki might accept — forward it.
        Err(e) => debug!(error = %e, "unparseable proxy body, forwarding as-is"),
    }

    // Forward byte-for-byte and relay the response unchanged — the proxy's
    // contract. Enrichment is a *separate* follow-up write, never a mutation of
    // what Yomitan sent or what it gets back.
    let (status, resp_bytes) = match forward(&state, body).await {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };

    match (added_note, new_note_id(&resp_bytes)) {
        (Some((req, anchor_ts)), Some(note_id)) => {
            let state = state.clone();
            // Detached: card creation must not wait on an LLM call or a capture.
            tokio::spawn(async move { enrich_added_note(&state, note_id, &req, anchor_ts).await });
        }
        (Some(_), None) => warn!(
            resp = %String::from_utf8_lossy(&resp_bytes),
            "proxy: addNote returned no note id (duplicate or error) — not enriching"
        ),
        _ => {}
    }

    let mut headers = cors_headers();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    (status, headers, resp_bytes).into_response()
}

/// The note id AnkiConnect returns from a successful `addNote`
/// (`{"result": 12345, "error": null}`), or `None` on a duplicate/error.
fn new_note_id(resp_bytes: &Bytes) -> Option<i64> {
    let json: Value = serde_json::from_slice(resp_bytes).ok()?;
    // AnkiConnect answers two ways. With `"version": 6` it wraps the reply in
    // `{"result": <id>, "error": null}`. Yomitan's addNote omits the version, so
    // AnkiConnect falls back to legacy mode and returns the *bare* result — the
    // note id on success, a bare `null` on a duplicate/failure. Handle both, or
    // every Yomitan mine silently skips enrichment.
    if json.is_object() {
        if !json.get("error").map(Value::is_null).unwrap_or(true) {
            return None;
        }
        json.get("result").and_then(Value::as_i64)
    } else {
        json.as_i64()
    }
}

/// Fire vn-capture for audio + picture and write CompactDef onto a freshly
/// added note. Best-effort: any failure is logged, never surfaced, since the
/// card already exists and is usable without either.
///
/// **Nothing may be awaited in front of the capture.** A screenshot shows the
/// screen as it is when taken, so anything ahead of it puts the *next* line on
/// the card. (`anchor_ts` pins the audio window; the picture has no such
/// recourse.) But making CompactDef wait for the capture only moved the delay
/// onto CompactDef, which then landed ten seconds after the add.
///
/// So the LLM call runs *alongside* the capture with its write afterwards. The
/// two `updateNoteFields` stay strictly ordered — two concurrent writes to one
/// note are untested and there is nothing to gain by starting.
///
/// All of this happens behind a tab nobody is watching, so the chime at the end
/// is the only report, and it plays only when nothing failed.
async fn enrich_added_note(state: &AppState, note_id: i64, req: &Value, anchor_ts: f64) {
    // CompactDef: only when a target field and an API key are configured.
    let fields = req.pointer("/params/note/fields");
    let word = fields
        .and_then(|f| f.get(&state.anki_vocab_field))
        .and_then(Value::as_str)
        .map(crate::services::anki::clean_field)
        .unwrap_or_default();
    let raw_sentence = fields
        .and_then(|f| f.get(&state.anki_sentence_field))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let sentence = crate::services::anki::clean_field_keep_bold(raw_sentence);

    // The gloss is tagged on the spelling the page used, not on the headword —
    // 饐える reads RARE where its own sentence's すえた does not. The bold span
    // Yomitan leaves in the sentence is that spelling; the vocab field is the
    // fallback for a note that has no markers to parse.
    let target =
        crate::services::anki::bolded_span(raw_sentence).unwrap_or_else(|| word.clone());

    // Auto-capture: fold the mine button into the add. The note id and the
    // anchor both come from the add itself, so neither depends on what has
    // happened on screen or in Anki since.
    //
    // Reports whether the media actually landed. `ok: false` is the script's
    // normal way of saying a capture was not possible (a stale ring, no speech
    // on the clip), which is a real outcome for the card even though it is not
    // an error for us — so it counts as "did not fully succeed" for the chime.
    let capture = async {
        if !state.auto_capture_on_add {
            return true;
        }
        let target = crate::services::capture::Target {
            anchor_ts: Some(anchor_ts),
            note_id: Some(note_id),
        };
        match crate::services::capture::run(state, target).await {
            Ok(result) => {
                info!(note_id, result = %result, "auto-capture after add");
                result.get("ok").and_then(Value::as_bool) == Some(true)
            }
            Err(e) => {
                warn!(note_id, error = %e, "auto-capture after add failed");
                false
            }
        }
    };

    // Whether there is a definition to ask for at all, decided before anything
    // is awaited so the capture never waits on a call that was not going to
    // happen.
    let api_key =
        if state.anki_compact_def_field.is_empty() || target.is_empty() || sentence.is_empty() {
            warn!(
                note_id,
                word = %word,
                target = %target,
                target_empty = target.is_empty(),
                sentence_empty = sentence.is_empty(),
                compact_field_empty = state.anki_compact_def_field.is_empty(),
                "enrich: skipped CompactDef — empty target, sentence, or field"
            );
            None
        } else if state.anthropic_api_key.is_none() {
            warn!(note_id, "enrich: no Anthropic API key; skipping CompactDef");
            None
        } else {
            state.anthropic_api_key.as_deref()
        };

    // No definition to fetch: the capture is the whole of the enrichment, so it
    // alone decides whether this card came out complete.
    let Some(api_key) = api_key else {
        if capture.await {
            crate::services::chime::mine_complete();
        }
        return;
    };

    let (def, captured) = tokio::join!(
        crate::services::compactdef::compact_def(&state.http, api_key, &target, &sentence),
        capture,
    );

    let defined = match def {
        Ok(def) if !def.is_empty() => {
            // Verified, not fire-and-forget: see `update_note_field_verified`.
            match crate::services::anki::update_note_field_verified(
                &state.http,
                &state.anki_url,
                note_id,
                &state.anki_compact_def_field,
                &def,
            )
            .await
            {
                Ok(()) => {
                    info!(note_id, word = %word, "CompactDef written and verified");
                    true
                }
                Err(e) => {
                    warn!(note_id, error = %e, "CompactDef write failed");
                    false
                }
            }
        }
        Ok(_) => {
            warn!(note_id, word, "CompactDef came back empty");
            false
        }
        Err(e) => {
            warn!(note_id, word, error = %e, "CompactDef generation failed");
            false
        }
    };

    // The card is complete: media attached and the definition verified onto the
    // note. Anything less stays silent, so the sound means one thing only.
    if captured && defined {
        crate::services::chime::mine_complete();
    }
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
pub(crate) async fn record(state: &AppState, term: &str) {
    let settings = match db::load_settings(&state.local).await {
        Ok(s) => s,
        Err(e) => {
            // Without settings there is no window to test against and no work
            // to stamp. Dropping the lookup is the safe half of the trade: a
            // missed lookup understates a rate, a stray one corrupts a work.
            warn!(error = %e, term, "settings unavailable, not recording lookup");
            return;
        }
    };

    match db::line_within(&state.knowledge, now_ts(), settings.session_gap_secs).await {
        Ok(true) => {}
        Ok(false) => {
            debug!(term, "lookup outside a reading session, not recorded");
            return;
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
        Ok(true) => debug!(term, "lookup recorded"),
        Ok(false) => debug!(term, "lookup deduped"),
        // Counting lookups must never break mining: log and forward anyway.
        Err(e) => warn!(error = %e, term, "failed to record lookup"),
    }
}

/// Forward to AnkiConnect and return `(status, response bytes)`, so the caller
/// can relay it unchanged and inspect it. A transport error comes back as a
/// ready-made `Response` in `Err`, to pass straight through.
async fn forward(state: &AppState, body: Bytes) -> Result<(StatusCode, Bytes), Response> {
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
            return Err((StatusCode::BAD_GATEWAY, cors_headers(), e.to_string()).into_response());
        }
    };

    let status = resp.status();
    match resp.bytes().await {
        Ok(bytes) => Ok((status, bytes)),
        Err(e) => {
            warn!(error = %e, "AnkiConnect response unreadable");
            Err((StatusCode::BAD_GATEWAY, cors_headers(), e.to_string()).into_response())
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
    fn new_note_id_reads_successful_add() {
        let ok = Bytes::from(r#"{"result": 1784796207918, "error": null}"#);
        assert_eq!(new_note_id(&ok), Some(1784796207918));
    }

    #[test]
    fn new_note_id_reads_legacy_bare_response() {
        // Yomitan omits "version": 6, so AnkiConnect returns the bare id...
        assert_eq!(
            new_note_id(&Bytes::from("1784933649618")),
            Some(1784933649618)
        );
        // ...and a bare null on a duplicate/failure.
        assert_eq!(new_note_id(&Bytes::from("null")), None);
    }

    #[test]
    fn new_note_id_ignores_duplicate_and_error() {
        // Duplicate: AnkiConnect returns null result with an error string.
        let dup = Bytes::from(
            r#"{"result": null, "error": "cannot create note because it is a duplicate"}"#,
        );
        assert_eq!(new_note_id(&dup), None);
        // No result at all.
        let empty = Bytes::from(r#"{"error": null}"#);
        assert_eq!(new_note_id(&empty), None);
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
