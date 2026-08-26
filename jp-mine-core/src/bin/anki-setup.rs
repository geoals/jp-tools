//! `anki-setup` — is Anki ready to take a card, and set it up if not.
//!
//!     anki-setup check           report the note type and its fields
//!     anki-setup install-lapis   download the Lapis note type and import it
//!
//! It lives here because the field map does: `AnkiConfig` is what the exporter
//! writes through, so a check against any other list would drift from the cards
//! actually being made.
//!
//! Lapis is downloaded from its own release rather than vendored. It is
//! GPL-3.0 and it moves, and AnkiConnect can import an `.apkg` directly, so a
//! copy here would be a second version to keep current for no gain.

use std::process::ExitCode;

use jp_mine_core::config::AnkiConfig;
use serde_json::{Value, json};

const LAPIS_RELEASE: &str = "https://api.github.com/repos/donkuri/lapis/releases/latest";
const LAPIS_ASSET: &str = "Lapis.apkg";

fn anki_url() -> String {
    std::env::var("KOTODEX_ANKI_URL").unwrap_or_else(|_| "http://127.0.0.1:8765".into())
}

async fn anki(client: &reqwest::Client, action: &str, params: Value) -> Result<Value, String> {
    let body = json!({ "action": action, "version": 6, "params": params });
    let resp = client
        .post(anki_url())
        .json(&body)
        .send()
        .await
        .map_err(|_| format!("AnkiConnect is not answering on {}", anki_url()))?;
    let v: Value = resp.json().await.map_err(|e| e.to_string())?;
    // AnkiConnect answers 200 with the error in the body, so the status says
    // nothing — `error` is the only place a refusal appears.
    match v.get("error") {
        Some(Value::String(e)) => Err(e.clone()),
        _ => Ok(v["result"].clone()),
    }
}

async fn check(client: &reqwest::Client) -> ExitCode {
    let config = AnkiConfig::from_env();

    let models = match anki(client, "modelNames", json!({})).await {
        Ok(v) => v,
        Err(e) => {
            println!("✗ {e}");
            println!("  install Anki, add the AnkiConnect add-on, and leave Anki running");
            return ExitCode::FAILURE;
        }
    };
    let models: Vec<String> = serde_json::from_value(models).unwrap_or_default();
    println!("✓ AnkiConnect is answering ({} note types)", models.len());

    if !models.contains(&config.model_name) {
        println!("✗ note type {} is not in this collection", config.model_name);
        if config.model_name == "Lapis" {
            println!("  anki-setup install-lapis  — downloads it and imports it");
        } else {
            println!("  set KOTODEX_ANKI_MODEL to one you have, or create it in Anki");
        }
        return ExitCode::FAILURE;
    }

    let fields = anki(
        client,
        "modelFieldNames",
        json!({ "modelName": config.model_name }),
    )
    .await;
    let fields: Vec<String> = match fields {
        Ok(v) => serde_json::from_value(v).unwrap_or_default(),
        Err(e) => {
            println!("✗ could not read {}'s fields: {e}", config.model_name);
            return ExitCode::FAILURE;
        }
    };

    let missing: Vec<_> = config
        .configured_fields()
        .into_iter()
        .filter(|(_, name)| !fields.iter().any(|f| f == name))
        .collect();

    if missing.is_empty() {
        println!(
            "✓ note type {} has every field the exporter writes",
            config.model_name
        );
        return ExitCode::SUCCESS;
    }

    println!("✗ note type {} is missing fields:", config.model_name);
    for (what, name) in &missing {
        println!("    {name}  — the {what}");
    }
    println!("  it has: {}", fields.join(", "));
    println!("  add the missing ones in Anki, or rename each to a field it has:");
    for (what, _) in &missing {
        println!("    {}=<field>   # {what}, or empty for none", env_var_for(what));
    }
    ExitCode::FAILURE
}

/// Which variable renames a field. Kept beside the check because that is the
/// one place the answer is needed, and a wrong name here is a fix that does
/// nothing.
fn env_var_for(what: &str) -> &'static str {
    match what {
        "headword" => "KOTODEX_ANKI_FIELD_VOCAB",
        "definition" => "KOTODEX_ANKI_FIELD_DEFINITION",
        "gloss" => "KOTODEX_ANKI_FIELD_COMPACT_DEF",
        "sentence" => "KOTODEX_ANKI_FIELD_SENTENCE",
        "image" => "KOTODEX_ANKI_FIELD_IMAGE",
        "audio" => "KOTODEX_ANKI_FIELD_AUDIO",
        "source" => "KOTODEX_ANKI_FIELD_SOURCE",
        "furigana" => "KOTODEX_ANKI_FIELD_FURIGANA",
        "reading" => "KOTODEX_ANKI_FIELD_READING",
        "word audio" => "KOTODEX_ANKI_FIELD_VOCAB_AUDIO",
        "pitch position" => "KOTODEX_ANKI_FIELD_PITCH_NUM",
        "pitch pattern" => "KOTODEX_ANKI_FIELD_PITCH_PATTERN",
        "frequency" => "KOTODEX_ANKI_FIELD_FREQUENCY",
        "frequency sort" => "KOTODEX_ANKI_FIELD_FREQ_SORT",
        _ => "KOTODEX_ANKI_FIELD_?",
    }
}

async fn install_lapis(client: &reqwest::Client) -> ExitCode {
    // GitHub's API refuses a request with no User-Agent.
    let release: Value = match client
        .get(LAPIS_RELEASE)
        .header("User-Agent", "kotodex")
        .send()
        .await
        .and_then(|r| r.error_for_status())
    {
        Ok(r) => match r.json().await {
            Ok(v) => v,
            Err(e) => {
                println!("✗ could not read the Lapis release: {e}");
                return ExitCode::FAILURE;
            }
        },
        Err(e) => {
            println!("✗ could not reach the Lapis release: {e}");
            return ExitCode::FAILURE;
        }
    };

    let url = release["assets"]
        .as_array()
        .and_then(|assets| {
            assets
                .iter()
                .find(|a| a["name"] == LAPIS_ASSET)
                .and_then(|a| a["browser_download_url"].as_str())
        })
        .map(str::to_string);
    let Some(url) = url else {
        println!("✗ the latest Lapis release has no {LAPIS_ASSET}");
        println!("  get it by hand: https://github.com/donkuri/lapis/releases");
        return ExitCode::FAILURE;
    };

    println!("downloading Lapis {}", release["tag_name"].as_str().unwrap_or("?"));
    let bytes = match client.get(&url).send().await {
        Ok(r) => match r.bytes().await {
            Ok(b) => b,
            Err(e) => {
                println!("✗ download failed: {e}");
                return ExitCode::FAILURE;
            }
        },
        Err(e) => {
            println!("✗ download failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    // importPackage takes a path, not the bytes, and Anki opens it itself — so
    // it has to land somewhere Anki can read. /tmp is not that place: a Flatpak
    // Anki has its own, and the import fails with a file-not-found naming a
    // path that plainly exists. Anki's own profile directory always works, and
    // asking for the media directory is how to find it.
    let path = import_dir(client).await.join("kotodex-lapis.apkg");
    if let Err(e) = std::fs::write(&path, &bytes) {
        println!("✗ could not write {}: {e}", path.display());
        return ExitCode::FAILURE;
    }

    let arg = json!({ "path": path.to_string_lossy() });

    // The silent import first. It is gone in current Anki — the importer
    // AnkiConnect calls was replaced, and the refusal comes back as an
    // exception with an empty message — but it still works on older ones, and
    // when it works nothing appears on screen.
    if anki(client, "importPackage", arg.clone()).await.is_ok() && has_lapis(client).await {
        let _ = std::fs::remove_file(&path);
        println!("✓ imported. Lapis brings its own deck; cards still go to your own.");
        return check(client).await;
    }

    // Anki's own import dialog, opened on the file. One click, and it is the
    // only path that works on every version.
    match anki(client, "guiImportFile", arg).await {
        Ok(_) => {
            println!("→ Anki's import dialog is open on Lapis. Click Import, then:");
            println!("    anki-setup check");
            println!("  the file can be deleted afterwards: {}", path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("✗ Anki would not import it: {e}");
            println!("  do it by hand — Anki, File, Import: {}", path.display());
            ExitCode::FAILURE
        }
    }
}

async fn has_lapis(client: &reqwest::Client) -> bool {
    let models = anki(client, "modelNames", json!({})).await.unwrap_or_default();
    let models: Vec<String> = serde_json::from_value(models).unwrap_or_default();
    models.iter().any(|m| m == "Lapis")
}

/// A directory Anki can read. Its profile folder, which is the media
/// directory's parent — that is the one path that is inside the sandbox when
/// there is one, and outside it when there is not.
async fn import_dir(client: &reqwest::Client) -> std::path::PathBuf {
    let media = anki(client, "getMediaDirPath", json!({}))
        .await
        .ok()
        .and_then(|v| v.as_str().map(std::path::PathBuf::from));
    match media.as_ref().and_then(|m| m.parent()) {
        Some(profile) if profile.is_dir() => profile.to_path_buf(),
        // An AnkiConnect too old to answer, or a path this process cannot see.
        // Temp is the best guess left, and the error names it if Anki disagrees.
        _ => std::env::temp_dir(),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // The same file kotodex-server loads, so the check is against the field map the
    // exporter will actually use rather than the Lapis defaults.
    dotenvy::dotenv().ok();
    let client = reqwest::Client::new();
    match std::env::args().nth(1).as_deref() {
        Some("check") | None => check(&client).await,
        Some("install-lapis") => install_lapis(&client).await,
        Some(other) => {
            println!("unknown command: {other}");
            println!("usage: anki-setup [check | install-lapis]");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every field the check can report missing has to name the variable that
    /// renames it. A field added to `AnkiConfig` and missed here prints
    /// `KOTODEX_ANKI_FIELD_?`, which is a fix that does nothing.
    #[test]
    fn every_configured_field_has_a_variable() {
        for (what, _) in AnkiConfig::default().configured_fields() {
            assert_ne!(env_var_for(what), "KOTODEX_ANKI_FIELD_?", "{what}");
        }
    }
}
