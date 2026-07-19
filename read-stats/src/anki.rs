//! Read-only AnkiConnect client: probe for a reachable instance (dashboard
//! client first — mining is phone-first in this stack — then the configured
//! fallback) and snapshot the mined deck's vocab field.

use std::net::IpAddr;
use std::time::Duration;

use serde_json::{Value, json};
use tracing::debug;

use crate::db::AnkiNote;
use crate::error::AppError;

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// notesInfo batch size — AnkiconnectAndroid chokes on very large requests.
const NOTES_CHUNK: usize = 500;

async fn call(
    client: &reqwest::Client,
    url: &str,
    action: &str,
    params: Value,
) -> Result<Value, AppError> {
    let body = json!({ "action": action, "version": 6, "params": params });
    let resp = client
        .post(url)
        .timeout(REQUEST_TIMEOUT)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("AnkiConnect '{action}' failed: {e}")))?;
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
/// (phone with AnkiconnectAndroid), then the configured fallback (desktop).
pub fn candidate_urls(client_ip: Option<IpAddr>, fallback: &str) -> Vec<String> {
    let mut urls = Vec::new();
    if let Some(ip) = client_ip {
        if !ip.is_loopback() {
            urls.push(match ip {
                IpAddr::V4(v4) => format!("http://{v4}:8765"),
                IpAddr::V6(v6) => format!("http://[{v6}]:8765"),
            });
        }
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

/// Strip HTML tags and surrounding whitespace from a field value.
fn clean_field(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;
    for c in raw.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// Fetch (note_id, vocab) for every note in the deck.
pub async fn fetch_deck_vocab(
    client: &reqwest::Client,
    url: &str,
    deck: &str,
    vocab_field: &str,
) -> Result<Vec<AnkiNote>, AppError> {
    let ids_val = call(client, url, "findNotes", json!({ "query": format!("deck:\"{deck}\"") })).await?;
    let ids: Vec<i64> = ids_val
        .as_array()
        .ok_or_else(|| AppError::Upstream("unexpected findNotes response".into()))?
        .iter()
        .filter_map(Value::as_i64)
        .collect();

    let mut notes = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(NOTES_CHUNK) {
        let info = call(client, url, "notesInfo", json!({ "notes": chunk })).await?;
        let arr = info
            .as_array()
            .ok_or_else(|| AppError::Upstream("unexpected notesInfo response".into()))?;
        for note in arr {
            let Some(id) = note["noteId"].as_i64() else { continue };
            let vocab = clean_field(note["fields"][vocab_field]["value"].as_str().unwrap_or(""));
            if !vocab.is_empty() {
                notes.push(AnkiNote { note_id: id, vocab });
            }
        }
    }
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_prefer_client_then_fallback() {
        let urls = candidate_urls(Some("192.168.1.7".parse().unwrap()), "http://localhost:8765");
        assert_eq!(urls, vec!["http://192.168.1.7:8765", "http://localhost:8765"]);
        // loopback client collapses into the fallback alone
        let urls = candidate_urls(Some("127.0.0.1".parse().unwrap()), "http://localhost:8765");
        assert_eq!(urls, vec!["http://localhost:8765"]);
        // client that IS the fallback isn't probed twice
        let urls = candidate_urls(Some("192.168.1.7".parse().unwrap()), "http://192.168.1.7:8765");
        assert_eq!(urls, vec!["http://192.168.1.7:8765"]);
    }

    #[test]
    fn clean_field_strips_tags() {
        assert_eq!(clean_field("隔週"), "隔週");
        assert_eq!(clean_field(" <b>隔週</b> "), "隔週");
        assert_eq!(clean_field("<img src=\"x.jpg\">"), "");
    }
}
