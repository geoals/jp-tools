//! Read-only AnkiConnect client: probe for a reachable instance (the dashboard
//! client first, then the configured fallback) and snapshot the mined deck's
//! vocab field.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::db::AnkiNote;
use crate::error::AppError;

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// notesInfo batch size — AnkiconnectAndroid chokes on very large requests.
const NOTES_CHUNK: usize = 500;
/// cardsInfo batch size. Smaller than `NOTES_CHUNK` because each card comes
/// back with its rendered question, answer and CSS.
const CARDS_CHUNK: usize = 200;

async fn call(
    client: &reqwest::Client,
    url: &str,
    action: &str,
    params: Value,
) -> Result<Value, AppError> {
    let body = json!({ "action": action, "version": 6, "params": params });
    // AnkiConnect closes the TCP connection after every response, so a pooled
    // one is already dead when the next call reaches for it and the send fails
    // before anything is transmitted. One retry is the whole fix: the second
    // attempt opens a fresh connection. Safe to repeat because it failed on the
    // way out — Anki never saw it.
    let mut attempt = 0;
    let resp = loop {
        match client
            .post(url)
            .timeout(REQUEST_TIMEOUT)
            .json(&body)
            .send()
            .await
        {
            Ok(resp) => break resp,
            Err(e) if attempt == 0 && e.is_request() => attempt += 1,
            Err(e) => {
                return Err(AppError::Upstream(format!(
                    "AnkiConnect '{action}' failed: {e}"
                )));
            }
        }
    };
    let body: Value = resp
        .json()
        .await
        .map_err(|e| AppError::Upstream(format!("AnkiConnect '{action}' unreadable: {e}")))?;
    match body.get("error") {
        Some(Value::Null) | None => Ok(body["result"].clone()),
        Some(err) => Err(AppError::Upstream(format!(
            "AnkiConnect error on '{action}': {err}"
        ))),
    }
}

async fn reachable(client: &reqwest::Client, url: &str) -> bool {
    let body = json!({ "action": "version", "version": 6 });
    matches!(
        client.post(url).timeout(PROBE_TIMEOUT).json(&body).send().await,
        Ok(resp) if resp.status().is_success()
    )
}

/// Candidate AnkiConnect URLs in preference order: the dashboard client's IP
/// (a device running AnkiconnectAndroid), then the configured fallback.
pub fn candidate_urls(client_ip: Option<IpAddr>, fallback: &str) -> Vec<String> {
    let mut urls = Vec::new();
    if let Some(ip) = client_ip
        && !ip.is_loopback()
    {
        urls.push(match ip {
            IpAddr::V4(v4) => format!("http://{v4}:8765"),
            IpAddr::V6(v6) => format!("http://[{v6}]:8765"),
        });
    }
    if !urls.contains(&fallback.to_string()) {
        urls.push(fallback.to_string());
    }
    urls
}

/// First reachable candidate, if any.
pub async fn pick_url(
    client: &reqwest::Client,
    client_ip: Option<IpAddr>,
    fallback: &str,
) -> Option<String> {
    for url in candidate_urls(client_ip, fallback) {
        if reachable(client, &url).await {
            debug!(%url, "AnkiConnect reachable");
            return Some(url);
        }
    }
    None
}

/// Put a native recording of the word into Anki's media folder and return the
/// `[sound:...]` tag for the card's audio field.
///
/// Yomitan pulls its vocabulary audio from the local-audio add-on; the overlay
/// has no Yomitan, so it asks the same add-on itself and stores the result the
/// way Yomitan would. Empty string when there is no recording, when the add-on
/// is not running, or when Anki rejects the file — the card is worth adding
/// without audio, and a failure here must not cost it.
pub async fn store_vocab_audio(
    client: &reqwest::Client,
    url: &str,
    term: &str,
    reading: &str,
) -> String {
    let recording = match jp_mine_core::localaudio::fetch(client, term, reading).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            debug!(term, reading, "no local audio for this word");
            return String::new();
        }
        Err(e) => {
            warn!(term, reading, error = %e, "local audio lookup failed");
            return String::new();
        }
    };
    let stored = call(
        client,
        url,
        "storeMediaFile",
        json!({ "filename": recording.filename, "data": recording.data }),
    )
    .await;
    match stored {
        // The name Anki answers with, not the one it was sent: Anki normalizes
        // what it stores, and a card pointing at the name we asked for plays
        // nothing.
        Ok(Value::String(stored_name)) => {
            debug!(term, source = %recording.source, stored_name, "vocabulary audio attached");
            format!("[sound:{stored_name}]")
        }
        Ok(other) => {
            warn!(term, %other, "storeMediaFile returned no filename");
            String::new()
        }
        Err(e) => {
            warn!(term, error = %e, "storing vocabulary audio failed");
            String::new()
        }
    }
}

/// Set fields on an existing note. Used by the AnkiConnect proxy to write
/// CompactDef onto a note Yomitan just added.
pub async fn update_note_fields(
    client: &reqwest::Client,
    url: &str,
    note_id: i64,
    fields: Value,
) -> Result<(), AppError> {
    call(
        client,
        url,
        "updateNoteFields",
        json!({ "note": { "id": note_id, "fields": fields } }),
    )
    .await
    .map(|_| ())
}

/// Set one field and read it back, so a write that did not stick is reported
/// rather than logged as a success.
///
/// `updateNoteFields` answering `{"result": null, "error": null}` means Anki
/// accepted the request, *not* that the note still holds the value afterwards.
/// If the note is open in Anki's editor, the editor's own save writes its
/// in-memory copy over anything AnkiConnect changed in the meantime — observed
/// on a card whose CompactDef arrived four seconds after the mine, while the
/// note was open.
///
/// Writing a second time is deliberately not attempted: the editor would still
/// be open a second later and would clobber that too, so the retry would buy
/// nothing but another API call. Detection is the whole point — reopen the note
/// after the definition lands, or give it a few seconds before opening it.
pub async fn update_note_field_verified(
    client: &reqwest::Client,
    url: &str,
    note_id: i64,
    field: &str,
    value: &str,
) -> Result<(), AppError> {
    update_note_fields(client, url, note_id, json!({ field: value })).await?;
    match note_field(client, url, note_id, field).await {
        Ok(stored) if !stored.trim().is_empty() => Ok(()),
        Ok(_) => Err(AppError::Upstream(format!(
            "{field} is empty on note {note_id} despite a successful write — was the note open in Anki's editor?"
        ))),
        // Read-back failed: the write itself did not error, so nothing is known
        // against it. Note the blind spot and take the write at its word.
        Err(e) => {
            warn!(note_id, field, error = %e, "could not verify field write");
            Ok(())
        }
    }
}

/// The current value of one field on one note.
async fn note_field(
    client: &reqwest::Client,
    url: &str,
    note_id: i64,
    field: &str,
) -> Result<String, AppError> {
    let resp = call(client, url, "notesInfo", json!({ "notes": [note_id] })).await?;
    Ok(resp
        .pointer(&format!("/0/fields/{field}/value"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

/// The text Yomitan bolded inside a sentence field — the target word *as the
/// page actually spelt it*, conjugated and in whatever orthography the author
/// used.
///
/// This is the only place the surface form survives: the vocab field holds the
/// dictionary headword (饐える) while the sentence says すえた, and the
/// CompactDef call is tagged on the latter. Yomitan's card template wraps the
/// match in `<b>`, so the span is a parse rather than a re-tokenization.
///
/// Returns `None` when there is no bold span — a hand-made note, or a template
/// without the markers — leaving the caller to fall back to the headword.
pub fn bolded_span(raw: &str) -> Option<String> {
    let start = raw.find("<b>")? + "<b>".len();
    let end = raw[start..].find("</b>")? + start;
    let span = clean_field(&raw[start..end]);
    (!span.is_empty()).then_some(span)
}

/// `clean_field`, but keeping Yomitan's `<b>` markers around the target word.
///
/// The CompactDef prompt is given no headword, so the bold is how the model
/// finds which span of the sentence it is glossing.
pub fn clean_field_keep_bold(raw: &str) -> String {
    let marked = raw.replace("<b>", "\u{1}").replace("</b>", "\u{2}");
    clean_field(&marked)
        .replace('\u{1}', "<b>")
        .replace('\u{2}', "</b>")
}

/// Strip HTML tags and surrounding whitespace from a field value.
///
/// A ruby annotation's *reading* goes with the tags: `<rt>` and `<rp>` hold
/// furigana, which is a gloss on the spelling rather than part of it. Keeping
/// their text interleaves the reading into the word — 節穴 comes through as
/// 節ふし穴, which is not a spelling anything is written in, and it is the
/// string CompactDef would be told to rate "as it is written".
pub fn clean_field(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut tag = String::new();
    let mut in_tag = false;
    let mut in_reading = false;
    for c in raw.chars() {
        match c {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' => {
                in_tag = false;
                let name = tag.trim_start_matches('/');
                let name = name.split([' ', '\t', '\n']).next().unwrap_or("");
                if name.eq_ignore_ascii_case("rt") || name.eq_ignore_ascii_case("rp") {
                    in_reading = !tag.starts_with('/');
                }
            }
            c if in_tag => tag.push(c),
            c if !in_reading => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// Note ids matching an AnkiConnect search query.
async fn find_notes(
    client: &reqwest::Client,
    url: &str,
    query: &str,
) -> Result<Vec<i64>, AppError> {
    let ids_val = call(client, url, "findNotes", json!({ "query": query })).await?;
    Ok(ids_val
        .as_array()
        .ok_or_else(|| AppError::Upstream("unexpected findNotes response".into()))?
        .iter()
        .filter_map(Value::as_i64)
        .collect())
}

/// (note_id, vocab) for a set of notes, chunked through `notesInfo`. Shared by
/// every caller that starts from a note-id list — the whole deck, or a
/// `findNotes` search already narrowed to a queue.
async fn notes_vocab(
    client: &reqwest::Client,
    url: &str,
    ids: &[i64],
    vocab_field: &str,
) -> Result<Vec<AnkiNote>, AppError> {
    let mut notes = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(NOTES_CHUNK) {
        let info = call(client, url, "notesInfo", json!({ "notes": chunk })).await?;
        let arr = info
            .as_array()
            .ok_or_else(|| AppError::Upstream("unexpected notesInfo response".into()))?;
        for note in arr {
            let Some(id) = note["noteId"].as_i64() else {
                continue;
            };
            let vocab = clean_field(note["fields"][vocab_field]["value"].as_str().unwrap_or(""));
            if !vocab.is_empty() {
                // The AnkiConnect client has no tokenizer; the caller fills
                // `headword` before the snapshot is stored.
                notes.push(AnkiNote {
                    note_id: id,
                    vocab,
                    headword: String::new(),
                });
            }
        }
    }
    Ok(notes)
}

/// The oldest note whose vocab field is exactly this word, or `None`.
///
/// The same duplicate check Yomitan runs before it offers to add — which is
/// why it is asked of Anki rather than of `anki_notes`: that table is a
/// snapshot, and a card mined ten seconds ago is not in it. Oldest, because the
/// note id is the creation time and the first card for a word is the one worth
/// opening.
///
/// Escaped for Anki's search syntax, where `"` and `*` and `_` are operators.
pub async fn find_note_for_vocab(
    client: &reqwest::Client,
    url: &str,
    vocab_field: &str,
    term: &str,
) -> Result<Option<i64>, AppError> {
    let escaped = term.replace('\\', "\\\\").replace('"', "\\\"");
    let ids = find_notes(client, url, &format!("\"{vocab_field}:{escaped}\"")).await?;
    Ok(ids.into_iter().min())
}

/// Open Anki's card browser on one note. What Yomitan's own "view added note"
/// does, and it raises the Anki window over the game.
pub async fn gui_browse(client: &reqwest::Client, url: &str, note_id: i64) -> Result<(), AppError> {
    call(
        client,
        url,
        "guiBrowse",
        json!({ "query": format!("nid:{note_id}") }),
    )
    .await
    .map(|_| ())
}

/// Fetch (note_id, vocab) for every note in the deck.
pub async fn fetch_deck_vocab(
    client: &reqwest::Client,
    url: &str,
    deck: &str,
    vocab_field: &str,
) -> Result<Vec<AnkiNote>, AppError> {
    let ids = find_notes(client, url, &format!("deck:\"{deck}\"")).await?;
    notes_vocab(client, url, &ids, vocab_field).await
}

/// One card's scheduling state. What Anki knows about a word and the reading
/// does not.
#[derive(Debug, Clone)]
pub struct CardStat {
    pub card_id: i64,
    pub note_id: i64,
    /// Current interval in days. Negative in Anki for learning cards measured
    /// in seconds; those are filtered out by `is_review` rather than converted.
    pub interval: i64,
    pub reps: i64,
    pub lapses: i64,
    pub suspended: bool,
    /// Card type: 2 is review, which is the only state a re-encounter can say
    /// anything about — a new card has never been asked.
    pub card_type: i64,
    /// Last modification, epoch seconds — the fallback cutoff for a card with
    /// no review log. **Not a stand-in for the last review**: it was tried, and
    /// on this collection something bulk-touched every card at once, which put
    /// 2,196 of 2,256 cards at the same "reviewed 9 days ago". `mod` moves on
    /// anything Anki does to a card, so [`fetch_deck_reviews`] is the real
    /// answer and this is only what is left when the log has nothing.
    pub modified: f64,
}

impl CardStat {
    pub fn is_review(&self) -> bool {
        self.card_type == 2 && self.interval > 0
    }
}

/// Scheduling state for every card in the deck.
///
/// `cardsInfo` also returns each card's rendered question and answer, which is
/// most of the payload and none of the point — hence the small chunks.
pub async fn fetch_deck_cards(
    client: &reqwest::Client,
    url: &str,
    deck: &str,
) -> Result<Vec<CardStat>, AppError> {
    let ids_val = call(
        client,
        url,
        "findCards",
        json!({ "query": format!("deck:\"{deck}\"") }),
    )
    .await?;
    let ids: Vec<i64> = ids_val
        .as_array()
        .ok_or_else(|| AppError::Upstream("unexpected findCards response".into()))?
        .iter()
        .filter_map(Value::as_i64)
        .collect();

    let mut cards = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(CARDS_CHUNK) {
        let info = call(client, url, "cardsInfo", json!({ "cards": chunk })).await?;
        let arr = info
            .as_array()
            .ok_or_else(|| AppError::Upstream("unexpected cardsInfo response".into()))?;
        for card in arr {
            let Some(note_id) = card["note"].as_i64() else {
                continue;
            };
            cards.push(CardStat {
                card_id: card["cardId"].as_i64().unwrap_or_default(),
                note_id,
                interval: card["interval"].as_i64().unwrap_or(0),
                reps: card["reps"].as_i64().unwrap_or(0),
                lapses: card["lapses"].as_i64().unwrap_or(0),
                suspended: card["queue"].as_i64() == Some(-1),
                card_type: card["type"].as_i64().unwrap_or(0),
                modified: card["mod"].as_f64().unwrap_or(0.0),
            });
        }
    }
    Ok(cards)
}

/// The last time each card in the deck was actually reviewed, epoch seconds.
///
/// One call for the whole deck: `cardReviews` answers with every review row
/// since `startID`, and the rows are `[review time ms, card id, ...]`. The
/// whole log is ~20k rows and comes back in well under a second, so it is not
/// worth paging by date — and a per-card window would need the log anyway.
pub async fn fetch_deck_reviews(
    client: &reqwest::Client,
    url: &str,
    deck: &str,
) -> Result<HashMap<i64, f64>, AppError> {
    let rows = call(
        client,
        url,
        "cardReviews",
        json!({ "deck": deck, "startID": 0 }),
    )
    .await?;
    let rows = rows
        .as_array()
        .ok_or_else(|| AppError::Upstream("unexpected cardReviews response".into()))?;
    let mut last: HashMap<i64, f64> = HashMap::new();
    for row in rows {
        let (Some(at_ms), Some(card_id)) = (row[0].as_f64(), row[1].as_i64()) else {
            continue;
        };
        let at = at_ms / 1000.0;
        last.entry(card_id)
            .and_modify(|prev| *prev = prev.max(at))
            .or_insert(at);
    }
    Ok(last)
}

/// Fetch (note_id, vocab) for notes past Anki's new/learning queues — the
/// deck's review pile, evidence the reader actually has the word rather than
/// merely having queued it.
pub async fn fetch_reviewed_deck_vocab(
    client: &reqwest::Client,
    url: &str,
    deck: &str,
    vocab_field: &str,
) -> Result<Vec<AnkiNote>, AppError> {
    let ids = find_notes(client, url, &format!("deck:\"{deck}\" -is:new -is:learn")).await?;
    notes_vocab(client, url, &ids, vocab_field).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_prefer_client_then_fallback() {
        let urls = candidate_urls(
            Some("192.168.1.7".parse().unwrap()),
            "http://localhost:8765",
        );
        assert_eq!(
            urls,
            vec!["http://192.168.1.7:8765", "http://localhost:8765"]
        );
        // loopback client collapses into the fallback alone
        let urls = candidate_urls(Some("127.0.0.1".parse().unwrap()), "http://localhost:8765");
        assert_eq!(urls, vec!["http://localhost:8765"]);
        // client that IS the fallback isn't probed twice
        let urls = candidate_urls(
            Some("192.168.1.7".parse().unwrap()),
            "http://192.168.1.7:8765",
        );
        assert_eq!(urls, vec!["http://192.168.1.7:8765"]);
    }

    #[test]
    fn bolded_span_is_the_written_surface() {
        assert_eq!(
            bolded_span("湿度が高く、<b>すえた</b>臭いがする。").as_deref(),
            Some("すえた")
        );
        // Furigana inside the span is markup, not spelling.
        assert_eq!(
            bolded_span("<b><ruby>節<rp>(</rp><rt>ふし</rt><rp>)</rp></ruby>穴</b>じゃない")
                .as_deref(),
            Some("節穴")
        );
        assert_eq!(bolded_span("markerless sentence"), None);
        assert_eq!(bolded_span("<b></b>"), None);
    }

    #[test]
    fn clean_field_keep_bold_keeps_only_the_markers() {
        assert_eq!(
            clean_field_keep_bold(" <div>疑心暗鬼に陥らせる<b>流言飛語</b>……。</div> "),
            "疑心暗鬼に陥らせる<b>流言飛語</b>……。"
        );
    }

    #[test]
    fn clean_field_strips_tags() {
        assert_eq!(clean_field("隔週"), "隔週");
        assert_eq!(clean_field(" <b>隔週</b> "), "隔週");
        assert_eq!(clean_field("<img src=\"x.jpg\">"), "");
    }
}
