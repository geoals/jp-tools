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

/// What a title search offers the reader to pick from.
#[derive(serde::Serialize)]
pub struct VnMatch {
    pub id: String,
    /// As Japanese writes it where VNDB has it, since that is what the reader is
    /// looking at on screen. Falls back to the romanized title.
    pub title: String,
    /// The other one, shown beside it — the two are how you tell two entries in
    /// the same series apart.
    pub alt_title: String,
    pub cover: String,
    /// Roughly how long it is, in hours, from VNDB's own votes. **Not a character
    /// count**: VNDB has none, so the progress bar still needs one pasted in.
    pub hours: Option<f64>,
}

/// Search VNDB by title.
///
/// The whole point of the work form: a reader knows what they are reading and
/// should not have to find its id on a website to say so.
pub async fn search(
    client: &reqwest::Client,
    query: &str,
    limit: usize,
) -> Result<Vec<VnMatch>, AppError> {
    #[derive(Deserialize)]
    struct SearchResponse {
        results: Vec<SearchResult>,
    }
    #[derive(Deserialize)]
    struct SearchResult {
        id: String,
        title: Option<String>,
        alttitle: Option<String>,
        image: Option<VnImage>,
        length_minutes: Option<f64>,
    }

    let body = serde_json::json!({
        "filters": ["search", "=", query],
        "fields": "id,title,alttitle,image.url,length_minutes",
        "results": limit,
        // Most-voted first: a search for a well-known title should not open on a
        // fan disc nobody has played.
        "sort": "votecount",
        "reverse": true,
    });
    let res: SearchResponse = client
        .post(API_VN)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("vndb search: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Upstream(format!("vndb search decode: {e}")))?;

    Ok(res
        .results
        .into_iter()
        .map(|r| {
            let romanized = r.title.unwrap_or_default();
            let original = r.alttitle.unwrap_or_default();
            // The Japanese spelling leads where there is one: it is what the game
            // window and the script say, so it is the title a reader recognises.
            let (title, alt_title) = if original.is_empty() {
                (romanized, String::new())
            } else {
                (original, romanized)
            };
            VnMatch {
                id: r.id,
                title,
                alt_title,
                cover: r.image.map(|i| i.url).unwrap_or_default(),
                hours: r.length_minutes.map(|m| m / 60.0),
            }
        })
        .collect())
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
/// since a Japanese script will not use them and a whole ASCII entry cannot
/// collide with a Japanese word.
///
/// **Whole is the operative word, and only a Japanese form is split.** VNDB's
/// aliases are prose as often as names — "Prison guard", "Old man", "Magical
/// Girl Riruru" — and splitting those on their spaces makes guard, man, Old and
/// Girl into people. A romanized form is taken as it stands.
pub async fn fetch_cast(client: &reqwest::Client, vndb_id: &str) -> Result<Vec<String>, AppError> {
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
            if form.chars().any(is_japanese_char) {
                for part in form.split_whitespace() {
                    names.push(part.to_string());
                }
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

/// Kana or kanji — what makes a form a name a Japanese script would write.
fn is_japanese_char(c: char) -> bool {
    jp_core::text::kana::is_all_kana(&c.to_string()) || jp_core::text::kanji::is_kanji(c)
}

/// Accept "v3144", "3144", or a vndb.org URL; return the canonical "v3144".
pub fn normalize_id(input: &str) -> Option<String> {
    let s = input.trim().trim_end_matches('/');
    let s = s.rsplit('/').next().unwrap_or(s);
    let digits = s.strip_prefix('v').unwrap_or(s);
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| format!("v{digits}"))
}

/// One-shot lookup of a VN's cover image URL.
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
