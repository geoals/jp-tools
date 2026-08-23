//! Adding a card to Anki, and everything that has to follow it.
//!
//! Every card path calls [`add_note`] — Yomitan's, through
//! [`crate::routes::ankiproxy`], and the overlay's, through
//! [`crate::routes::reader::mine`]. It is the seam the enrichment hangs off, so
//! a card is the same thing downstream whichever surface made it: the anchor
//! timestamp, vn-capture's audio and picture, CompactDef, the deck mirror and
//! the completion notification. Adding a third surface means calling this function, not routing
//! an HTTP request through another handler.
//!
//! The request stays [`Bytes`] rather than a `Value` because the proxy forwards
//! Yomitan's body byte-for-byte. Yomitan omits `"version": 6`, so AnkiConnect
//! answers in legacy mode with a bare id, which [`new_note_id`] has to handle;
//! re-serializing here would quietly change what goes on the wire.

use axum::body::Bytes;
use axum::http::{StatusCode, header};
use serde_json::Value;
use tracing::{info, warn};

use crate::app::AppState;
use crate::clock::now_ts;

/// Send an `addNote` to Anki and, if it took, enrich the card behind it.
///
/// Returns AnkiConnect's own `(status, body)` so the proxy can relay it
/// unchanged and the overlay can read the new id out of it.
pub async fn add_note(state: &AppState, body: Bytes) -> Result<(StatusCode, Bytes), String> {
    // Stamped before forwarding, because this is the last moment that still
    // answers "which line was on screen when the card was added" — everything
    // after it, Anki's own round-trip included, is time the reader can spend
    // reading on.
    let anchor_ts = now_ts();
    let req: Option<Value> = serde_json::from_slice(&body).ok();

    let (status, resp_bytes) = forward(state, body).await?;

    match (req, new_note_id(&resp_bytes)) {
        (Some(req), Some(note_id)) => {
            // Its own task, not a step inside `enrich_added_note`: nothing may
            // be awaited in front of the capture there, and this resolves a
            // ledger key through Sudachi.
            let (mirror_state, mirror_req) = (state.clone(), req.clone());
            tokio::spawn(
                async move { mirror_added_note(&mirror_state, note_id, &mirror_req).await },
            );
            let state = state.clone();
            // Detached: card creation must not wait on an LLM call or a capture.
            tokio::spawn(async move { enrich_added_note(&state, note_id, &req, anchor_ts).await });
        }
        (Some(_), None) => warn!(
            resp = %String::from_utf8_lossy(&resp_bytes),
            "addNote returned no note id (duplicate or error) — not enriching"
        ),
        _ => {}
    }

    Ok((status, resp_bytes))
}

/// The note id AnkiConnect returns from a successful `addNote`
/// (`{"result": 12345, "error": null}`), or `None` on a duplicate/error.
pub fn new_note_id(resp_bytes: &Bytes) -> Option<i64> {
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

/// Post a raw AnkiConnect body and return `(status, response bytes)`, so the
/// caller can relay it unchanged and inspect it.
pub async fn forward(state: &AppState, body: Bytes) -> Result<(StatusCode, Bytes), String> {
    // One retry on a send that never left, for the same reason
    // `services::anki::call` has one: AnkiConnect closes the connection after
    // every response, so the pooled one is already dead when the next request
    // reaches for it. Adding the audio lookup in front of the add made that
    // dead connection the normal case rather than an occasional one.
    let mut attempt = 0;
    let resp = loop {
        let sent = state
            .http
            .post(&state.anki_url)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.clone())
            .send()
            .await;
        match sent {
            Ok(r) => break r,
            Err(e) if attempt == 0 && e.is_request() => attempt += 1,
            Err(e) => {
                warn!(error = %e, url = %state.anki_url, "AnkiConnect unreachable");
                return Err(e.to_string());
            }
        }
    };

    let status = resp.status();
    match resp.bytes().await {
        Ok(bytes) => Ok((status, bytes)),
        Err(e) => {
            warn!(error = %e, "AnkiConnect response unreadable");
            Err(e.to_string())
        }
    }
}

/// Tell the deck mirror about the card that was just added.
///
/// Every cards-per-hour figure counts rows in `anki_notes`, and the only thing
/// that used to fill it was the dashboard page opening. An evening of mining in
/// the overlay left the mirror at whenever the page was last loaded, so the
/// cards existed in Anki and in no figure. Best-effort: a card that fails to
/// land here is still a card, and the next refresh picks it up.
async fn mirror_added_note(state: &AppState, note_id: i64, req: &Value) {
    let vocab = req
        .pointer("/params/note/fields")
        .and_then(|f| f.get(&state.anki_vocab_field))
        .and_then(Value::as_str)
        .map(crate::services::anki::clean_field)
        .unwrap_or_default();
    if vocab.is_empty() {
        return;
    }
    // The ledger key beside the card's own spelling, resolved the way the
    // refresh resolves it — a card says 検死 where the ledger row is 検屍, and
    // a raw spelling joins to nothing.
    let headword = crate::ingest::normalized_spellings(state, vec![vocab.clone()])
        .await
        .ok()
        .and_then(|mut k| k.pop())
        .unwrap_or_default();
    let note = crate::db::AnkiNote {
        note_id,
        vocab,
        headword,
    };
    if let Err(e) = crate::db::insert_anki_note(&state.knowledge, &note).await {
        warn!(note_id, error = %e, "could not add the card to the deck mirror");
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
/// All of this happens behind a tab nobody is watching, so the notification at the
/// end is the only report, and it is sent only when nothing failed.
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
    let target = crate::services::anki::bolded_span(raw_sentence).unwrap_or_else(|| word.clone());

    // Auto-capture: fold the mine button into the add. The note id and the
    // anchor both come from the add itself, so neither depends on what has
    // happened on screen or in Anki since.
    //
    // Reports whether the media actually landed. `ok: false` is the script's
    // normal way of saying a capture was not possible (a stale ring, no speech
    // on the clip), which is a real outcome for the card even though it is not
    // an error for us — so it counts as "did not fully succeed" for the report.
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

    // No key, but the dictionaries are right here: the note type shows the
    // gloss field as the card's headline, so it gets the first sense rather
    // than being left blank.
    let Some(api_key) = api_key else {
        let fallback = if state.anki_compact_def_field.is_empty() || word.is_empty() {
            None
        } else {
            jp_mine_core::card::first_sense(state.knowledge.pool(), &word, "")
                .await
                .unwrap_or_else(|e| {
                    warn!(note_id, error = %e, "enrich: first sense lookup failed");
                    None
                })
        };
        let defined = match fallback {
            Some(sense) => crate::services::anki::update_note_field_verified(
                &state.http,
                &state.anki_url,
                note_id,
                &state.anki_compact_def_field,
                &sense,
            )
            .await
            .inspect_err(|e| warn!(note_id, error = %e, "first-sense write failed"))
            .is_ok(),
            // No dictionary holds the word: the card is as complete as it can
            // be made, so this is not a failure to report.
            None => true,
        };
        if capture.await && defined {
            crate::services::notify::mine_complete(&word);
        }
        return;
    };

    let (def, captured) = tokio::join!(
        jp_mine_core::compactdef::compact_def(&state.http, api_key, &target, &sentence),
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
    // note. Anything less stays silent, so the notification means one thing only.
    if captured && defined {
        crate::services::notify::mine_complete(&word);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
