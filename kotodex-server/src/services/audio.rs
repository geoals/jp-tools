//! The Local Audio Server for Yomitan, running beside Anki.
//!
//! An Anki add-on that indexes NHK, 新明解, Forvo and JPod audio and answers
//! `GET /?term=&reading=` with a source list. It is the same server Yomitan
//! itself plays from, so a word sounds the same in the overlay as on the card.
//!
//! Proxied rather than called from the page, for two reasons that are both
//! about where it listens: it binds loopback only, so the overlay read off a
//! phone cannot reach it at all, and it sends no CORS headers, so even on this
//! machine the page's `fetch` would be refused. Serving it from here makes it
//! one more kotodex-server route, which is what every other thing the popup asks
//! for already is.

use serde::{Deserialize, Serialize};

use crate::app::AppState;
use crate::error::AppError;

/// One playable clip, as the popup receives it.
#[derive(Serialize)]
pub struct Source {
    /// The server's own label — `NHK16 ネ＼コ [1]`, `Forvo (akitomo)`. It
    /// carries the dictionary and the pitch, so it is worth showing as-is.
    pub name: String,
    /// The clip's path on the audio server, to be asked for back through
    /// [`fetch_clip`] — never the upstream URL, since the page may be a phone,
    /// for which `localhost:5050` is its own loopback.
    pub clip: String,
}

#[derive(Deserialize)]
struct UpstreamList {
    #[serde(rename = "audioSources", default)]
    audio_sources: Vec<UpstreamSource>,
}

#[derive(Deserialize)]
struct UpstreamSource {
    name: String,
    url: String,
}

/// What the audio server has for this word, in its own priority order.
///
/// An empty list is the ordinary answer for most words and is not an error —
/// the popup simply draws no button. So is the server being down: audio is the
/// one thing in the popup that is nice to have.
pub async fn sources(state: &AppState, term: &str, reading: &str) -> Vec<Source> {
    let url = format!("{}/", state.local_audio_url.trim_end_matches('/'));
    let response = state
        .http
        .get(&url)
        .query(&[("term", term), ("reading", reading)])
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await;
    let list = match response {
        Ok(r) => match r.json::<UpstreamList>().await {
            Ok(list) => list,
            Err(e) => {
                tracing::warn!(error = %e, "local audio server answered unreadably");
                return Vec::new();
            }
        },
        Err(e) => {
            tracing::debug!(error = %e, "no local audio server");
            return Vec::new();
        }
    };

    list.audio_sources
        .into_iter()
        .filter_map(|s| {
            Some(Source {
                name: s.name,
                clip: clip_path(&state.local_audio_url, &s.url)?,
            })
        })
        .collect()
}

/// The path part of an upstream clip URL, or `None` if it points anywhere else.
///
/// The proxy takes a path and puts this base back in front of it, so nothing
/// but this server is ever reachable through it. Checked on the way out as well
/// as on the way in: a source list naming another host is not something to
/// hand the page a kotodex-server URL for.
fn clip_path(base: &str, url: &str) -> Option<String> {
    let host = base.trim_end_matches('/');
    let rest = url.strip_prefix(host).or_else(|| {
        // The server names itself `localhost` in the URLs it returns while the
        // base is numeric, which is the same host by a different name.
        let swapped = host.replace("127.0.0.1", "localhost");
        url.strip_prefix(&swapped)
    })?;
    let rest = rest.trim_start_matches('/');
    safe_path(rest).map(str::to_string)
}

/// A relative path with nothing in it that could leave the audio server.
///
/// The proxy is a hole in the origin otherwise: `path=` reaching a scheme, a
/// host or a parent directory would make kotodex-server fetch whatever it was
/// pointed at and serve the answer as its own.
pub fn safe_path(path: &str) -> Option<&str> {
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.contains("..")
        || path.contains("://")
        || path.contains(['\\', '\r', '\n']);
    (!bad).then_some(path)
}

/// One clip's bytes and its content type.
pub async fn fetch_clip(state: &AppState, path: &str) -> Result<(String, Vec<u8>), AppError> {
    let path = safe_path(path)
        .ok_or_else(|| AppError::BadRequest("not an audio server path".to_string()))?;
    let url = format!("{}/{path}", state.local_audio_url.trim_end_matches('/'));
    let response = state
        .http
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("local audio server: {e}")))?;
    if !response.status().is_success() {
        return Err(AppError::NotFound);
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("audio/ogg")
        .to_string();
    let bytes = response
        .bytes()
        .await
        .map_err(|e| AppError::Upstream(format!("local audio server: {e}")))?;
    Ok((content_type, bytes.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clip_url_keeps_only_its_path() {
        assert_eq!(
            clip_path(
                "http://127.0.0.1:5050",
                "http://localhost:5050/nhk16/audio/1.opus"
            ),
            Some("nhk16/audio/1.opus".to_string())
        );
    }

    #[test]
    fn another_host_is_not_a_clip() {
        assert_eq!(
            clip_path("http://127.0.0.1:5050", "http://example.com/x.opus"),
            None
        );
    }

    #[test]
    fn a_path_may_not_escape_the_audio_server() {
        assert_eq!(safe_path("nhk16/audio/1.opus"), Some("nhk16/audio/1.opus"));
        assert_eq!(safe_path("../../etc/passwd"), None);
        assert_eq!(safe_path("/etc/passwd"), None);
        assert_eq!(safe_path("http://example.com/x"), None);
        assert_eq!(safe_path(""), None);
    }
}
