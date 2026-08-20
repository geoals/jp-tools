//! Vocabulary audio from the local-audio-yomichan Anki add-on.
//!
//! The add-on runs its own HTTP server beside AnkiConnect and answers a
//! `(term, reading)` with native recordings from NHK16, 新明解8, JPod and
//! Forvo. It is what Yomitan is configured to call for its audio sources, so a
//! card built here and a card built by Yomitan get audio from the same place.
//!
//! The reading is part of the query, not decoration: the accent of 空 depends
//! on which word it is, and a recording of the wrong one is worse than none.

/// Where the add-on listens. Fixed by the add-on, overridable for a machine
/// that runs Anki elsewhere.
pub fn base_url() -> String {
    std::env::var("JP_TOOLS_LOCAL_AUDIO_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:5050".to_string())
}

/// One recording the add-on offers.
#[derive(Clone, Debug)]
pub struct AudioSource {
    pub name: String,
    pub url: String,
}

/// Ask the add-on for recordings of `term` read as `reading`, best first.
///
/// The add-on's own ordering is the source priority configured in it, so the
/// first entry is the one Yomitan would have taken. Any failure — add-on not
/// installed, Anki not running, no recording — is `Ok(None)`: a card without
/// vocabulary audio is still a card.
pub async fn sources(
    client: &reqwest::Client,
    term: &str,
    reading: &str,
) -> Result<Vec<AudioSource>, String> {
    let url = format!("{}/", base_url());
    let resp = client
        .get(&url)
        .query(&[("term", term), ("reading", reading)])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("local audio server answered {}", resp.status()));
    }
    let body = resp.text().await.map_err(|e| e.to_string())?;
    Ok(parse(&body))
}

/// Read the add-on's reply, which is Yomitan's own `audioSourceList` shape.
/// An entry missing a URL is dropped rather than failing the lot.
fn parse(body: &str) -> Vec<AudioSource> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    json.get("audioSources")
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| {
                    Some(AudioSource {
                        name: e
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        url: e.get("url").and_then(|v| v.as_str())?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

impl AudioSource {
    /// The media filename to store this recording under, keyed on the word it
    /// is a recording *of* — one file per `(term, reading, source)` however
    /// many cards ask for it, and never a collision between two readings.
    ///
    /// The source is taken from the URL path rather than from `name`, which is
    /// a display string carrying the accent notation ("NHK16 アヤマ＼チ [3]").
    pub fn media_filename(&self, term: &str, reading: &str) -> String {
        let ext = self
            .url
            .rsplit('.')
            .next()
            .filter(|e| e.len() <= 4 && e.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or("mp3");
        format!("jp-tools_{term}_{reading}_{}.{ext}", self.source_id())
    }

    /// The add-on serves every file under `/<source>/…`, which is the stable
    /// id of the dictionary the recording came from.
    fn source_id(&self) -> &str {
        let after_scheme = match self.url.split_once("://") {
            Some((_, rest)) => rest,
            None => self.url.as_str(),
        };
        after_scheme
            .split('/')
            .nth(1)
            .filter(|segment| !segment.is_empty())
            .unwrap_or("local")
    }
}

/// A recording, downloaded and ready for AnkiConnect's `storeMediaFile`.
pub struct Recording {
    pub filename: String,
    /// The file itself, base64-encoded. Sent as bytes rather than as a URL for
    /// Anki to fetch, because Anki is not always on this machine and the
    /// add-on's server only listens on loopback.
    pub data: String,
    /// Which source it came from, for the log.
    pub source: String,
}

/// The best recording of `term` read as `reading`, downloaded.
///
/// `Ok(None)` means the add-on has no recording for this word — the common
/// case for rare words and names, and not an error.
pub async fn fetch(
    client: &reqwest::Client,
    term: &str,
    reading: &str,
) -> Result<Option<Recording>, String> {
    use base64::Engine;

    let Some(source) = sources(client, term, reading).await?.into_iter().next() else {
        return Ok(None);
    };
    let bytes = client
        .get(&source.url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    Ok(Some(Recording {
        filename: source.media_filename(term, reading),
        data: base64::engine::general_purpose::STANDARD.encode(&bytes),
        source: source.name,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_addons_reply() {
        let got = parse(
            r#"{"type":"audioSourceList","audioSources":[{"name":"NHK","url":"http://127.0.0.1:5050/nhk16/audio/x.aac"}]}"#,
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "NHK");
    }

    #[test]
    fn no_recording_is_an_empty_list_not_an_error() {
        assert!(parse(r#"{"type":"audioSourceList","audioSources":[]}"#).is_empty());
        assert!(parse("not json").is_empty());
    }

    #[test]
    fn filename_keeps_the_reading_apart() {
        let s = AudioSource {
            name: "NHK16 アヤマ＼チ [3]".into(),
            url: "http://127.0.0.1:5050/nhk16/audio/x.aac".into(),
        };
        assert_eq!(s.media_filename("空", "そら"), "jp-tools_空_そら_nhk16.aac");
        assert_ne!(
            s.media_filename("空", "そら"),
            s.media_filename("空", "から")
        );
    }

    #[test]
    fn filename_falls_back_when_the_url_has_no_extension() {
        let s = AudioSource {
            name: "Forvo (akitomo)".into(),
            url: "http://127.0.0.1:5050/forvo/akitomo/犬".into(),
        };
        assert!(s.media_filename("犬", "いぬ").ends_with(".mp3"));
    }
}
