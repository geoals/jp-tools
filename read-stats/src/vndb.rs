//! Minimal VNDB client (https://api.vndb.org/kana): metadata lookup and cover
//! download. No auth needed for public VN data.

use std::path::Path;

use serde::Deserialize;

use crate::error::AppError;

const API_VN: &str = "https://api.vndb.org/kana/vn";

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
