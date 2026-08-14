//! Minimal VNDB client (https://api.vndb.org/kana): metadata lookup and cover
//! download. No auth needed for public VN data.

use std::path::Path;

use serde::Deserialize;

use crate::error::AppError;

const API_VN: &str = "https://api.vndb.org/kana/vn";
const API_CHARACTER: &str = "https://api.vndb.org/kana/character";

#[derive(Deserialize)]
struct VnResponse {
    results: Vec<VnResult>,
}

#[derive(Deserialize)]
struct VnResult {
    image: Option<VnImage>,
}

#[derive(Deserialize)]
struct VnImage {
    url: String,
}

#[derive(Deserialize)]
struct CharacterResponse {
    results: Vec<Character>,
}

#[derive(Deserialize)]
struct Character {
    /// Romanized. Only used when a character has no Japanese spelling at all.
    name: String,
    /// As Japanese writes it — what a script actually contains.
    original: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
}

/// Search VNDB for a title, returning the first match's id.
pub async fn find_vn_id(client: &reqwest::Client, title: &str) -> Result<Option<String>, AppError> {
    let body = serde_json::json!({
        "filters": ["search", "=", title],
        "fields": "id",
        "results": 1,
    });
    #[derive(Deserialize)]
    struct IdResponse {
        results: Vec<IdResult>,
    }
    #[derive(Deserialize)]
    struct IdResult {
        id: String,
    }
    let res: IdResponse = client
        .post(API_VN)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("vndb search: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Upstream(format!("vndb search decode: {e}")))?;
    Ok(res.results.into_iter().next().map(|r| r.id))
}

/// Every spelling of every character in a VN, as a script might write them.
///
/// A full name is split on its space as well as kept whole: 飴井 カンナ is
/// written 飴井, カンナ or both, and each is a name wherever it appears.
/// Aliases come along for the same reason — オリヴィア・ベリー is called
/// オリーヴ throughout.
///
/// Romanized names are kept only where a character has no Japanese spelling,
/// since a Japanese script will not use them and an ASCII entry cannot collide
/// with a Japanese word.
pub async fn fetch_cast(
    client: &reqwest::Client,
    vndb_id: &str,
) -> Result<Vec<String>, AppError> {
    let body = serde_json::json!({
        "filters": ["vn", "=", ["id", "=", vndb_id]],
        "fields": "name,original,aliases",
        "results": 100,
    });
    let res: CharacterResponse = client
        .post(API_CHARACTER)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("vndb characters: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Upstream(format!("vndb characters decode: {e}")))?;

    let mut names = Vec::new();
    for c in res.results {
        let mut forms = c.aliases;
        match c.original {
            Some(original) => forms.push(original),
            None => forms.push(c.name),
        }
        for form in forms {
            for part in form.split_whitespace() {
                names.push(part.to_string());
            }
            let whole: String = form.split_whitespace().collect();
            if !whole.is_empty() {
                names.push(whole);
            }
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}

/// Accept "v3144", "3144", or a vndb.org URL; return the canonical "v3144".
pub fn normalize_id(input: &str) -> Option<String> {
    let s = input.trim().trim_end_matches('/');
    let s = s.rsplit('/').next().unwrap_or(s);
    let digits = s.strip_prefix('v').unwrap_or(s);
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| format!("v{digits}"))
}

/// One-shot lookup of a VN's cover image URL — the only thing we use VNDB for.
pub async fn fetch_cover_url(client: &reqwest::Client, vndb_id: &str) -> Result<String, AppError> {
    let body = serde_json::json!({
        "filters": ["id", "=", vndb_id],
        "fields": "image.url",
    });
    let resp = client
        .post(API_VN)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("vndb request failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Upstream(format!(
            "vndb returned {}",
            resp.status()
        )));
    }
    let parsed: VnResponse = resp
        .json()
        .await
        .map_err(|e| AppError::Upstream(format!("vndb response unreadable: {e}")))?;
    let vn = parsed
        .results
        .into_iter()
        .next()
        .ok_or_else(|| AppError::BadRequest(format!("no VN with id {vndb_id} on vndb")))?;
    vn.image
        .map(|i| i.url)
        .ok_or_else(|| AppError::BadRequest(format!("{vndb_id} has no cover on vndb")))
}

/// Download a cover into `covers_dir` as `<stem>.<ext>`; returns the filename.
pub async fn download_cover(
    client: &reqwest::Client,
    url: &str,
    covers_dir: &Path,
    stem: &str,
) -> Result<String, AppError> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("cover download failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Upstream(format!(
            "cover download returned {}",
            resp.status()
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::Upstream(format!("cover download failed: {e}")))?;

    let ext = url
        .rsplit('.')
        .next()
        .filter(|e| e.len() <= 4)
        .unwrap_or("jpg");
    let filename = format!("{stem}.{ext}");
    tokio::fs::create_dir_all(covers_dir)
        .await
        .map_err(|e| AppError::Upstream(format!("covers dir: {e}")))?;
    tokio::fs::write(covers_dir.join(&filename), &bytes)
        .await
        .map_err(|e| AppError::Upstream(format!("cover write: {e}")))?;
    Ok(filename)
}

#[cfg(test)]
mod tests {
    use super::normalize_id;

    #[test]
    fn normalize_accepts_common_forms() {
        assert_eq!(normalize_id("v3144").as_deref(), Some("v3144"));
        assert_eq!(normalize_id("3144").as_deref(), Some("v3144"));
        assert_eq!(
            normalize_id(" https://vndb.org/v3144 ").as_deref(),
            Some("v3144")
        );
        assert_eq!(
            normalize_id("https://vndb.org/v3144/").as_deref(),
            Some("v3144")
        );
        assert_eq!(normalize_id("subahibi"), None);
        assert_eq!(normalize_id(""), None);
        assert_eq!(normalize_id("v"), None);
    }
}
