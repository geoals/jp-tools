//! Getting the note type cards are made on into Anki.
//!
//! Lapis is downloaded from its own release rather than vendored. It is
//! GPL-3.0 and it moves, and AnkiConnect can import an `.apkg` directly, so a
//! copy here would be a second version to keep current for no gain.

use std::path::PathBuf;

use serde_json::{Value, json};

const LAPIS_RELEASE: &str = "https://api.github.com/repos/donkuri/lapis/releases/latest";
const LAPIS_ASSET: &str = "Lapis.apkg";

pub async fn anki(
    client: &reqwest::Client,
    url: &str,
    action: &str,
    params: Value,
) -> Result<Value, String> {
    let body = json!({ "action": action, "version": 6, "params": params });
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|_| format!("AnkiConnect is not answering on {url}"))?;
    let v: Value = resp.json().await.map_err(|e| e.to_string())?;
    // AnkiConnect answers 200 with the error in the body, so the status says
    // nothing — `error` is the only place a refusal appears.
    match v.get("error") {
        Some(Value::String(e)) => Err(e.clone()),
        _ => Ok(v["result"].clone()),
    }
}

pub enum Imported {
    Silently,
    AfterOneClick(PathBuf),
}

pub async fn install_lapis(client: &reqwest::Client, anki_url: &str) -> Result<Imported, String> {
    // GitHub's API refuses a request with no User-Agent.
    let release: Value = client
        .get(LAPIS_RELEASE)
        .header("User-Agent", "kotodex")
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("could not reach the Lapis release: {e}"))?
        .json()
        .await
        .map_err(|e| format!("could not read the Lapis release: {e}"))?;

    let url = release["assets"]
        .as_array()
        .and_then(|assets| {
            assets
                .iter()
                .find(|a| a["name"] == LAPIS_ASSET)
                .and_then(|a| a["browser_download_url"].as_str())
        })
        .ok_or_else(|| {
            format!(
                "the latest Lapis release has no {LAPIS_ASSET} — get it by hand from \
                 https://github.com/donkuri/lapis/releases"
            )
        })?
        .to_string();

    let bytes = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("download failed: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("download failed: {e}"))?;

    // importPackage takes a path, not the bytes, and Anki opens it itself — so
    // it has to land somewhere Anki can read. /tmp is not that place: a Flatpak
    // Anki has its own, and the import fails with a file-not-found naming a
    // path that plainly exists. Anki's own profile directory always works, and
    // asking for the media directory is how to find it.
    let path = import_dir(client, anki_url)
        .await
        .join("kotodex-lapis.apkg");
    std::fs::write(&path, &bytes)
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;

    let arg = json!({ "path": path.to_string_lossy() });

    // The silent import first: when it works nothing appears on screen. Current
    // Anki has no importer behind it and refuses with an exception carrying an
    // empty message, so the result is checked rather than trusted.
    if anki(client, anki_url, "importPackage", arg.clone())
        .await
        .is_ok()
        && has_lapis(client, anki_url).await
    {
        let _ = std::fs::remove_file(&path);
        return Ok(Imported::Silently);
    }

    // Anki's own import dialog, opened on the file. One click, and it is the
    // only path that works on every version.
    anki(client, anki_url, "guiImportFile", arg)
        .await
        .map(|_| Imported::AfterOneClick(path.clone()))
        .map_err(|e| {
            format!(
                "Anki would not import it: {e} — do it by hand through Anki, File, Import: {}",
                path.display()
            )
        })
}

async fn has_lapis(client: &reqwest::Client, anki_url: &str) -> bool {
    let models = anki(client, anki_url, "modelNames", json!({}))
        .await
        .unwrap_or_default();
    let models: Vec<String> = serde_json::from_value(models).unwrap_or_default();
    models.iter().any(|m| m == "Lapis")
}

/// A directory Anki can read. Its profile folder, which is the media
/// directory's parent — that is the one path that is inside the sandbox when
/// there is one, and outside it when there is not.
async fn import_dir(client: &reqwest::Client, anki_url: &str) -> PathBuf {
    let media = anki(client, anki_url, "getMediaDirPath", json!({}))
        .await
        .ok()
        .and_then(|v| v.as_str().map(PathBuf::from));
    match media.as_ref().and_then(|m| m.parent()) {
        Some(profile) if profile.is_dir() => profile.to_path_buf(),
        // An AnkiConnect too old to answer, or a path this process cannot see.
        // Temp is the best guess left, and the error names it if Anki disagrees.
        _ => std::env::temp_dir(),
    }
}
