//! `anki-setup` — is Anki ready to take a card, and set it up if not.
//!
//!     anki-setup check           report the note type and its fields
//!     anki-setup install-lapis   download the Lapis note type and import it
//!
//! It lives here because the field map does: `AnkiConfig` is what the exporter
//! writes through, so a check against any other list would drift from the cards
//! actually being made.

use std::process::ExitCode;

use jp_mine_core::config::AnkiConfig;
use jp_mine_core::note_type::{self, Imported};
use serde_json::{Value, json};

fn anki_url() -> String {
    std::env::var("KOTODEX_ANKI_URL").unwrap_or_else(|_| "http://127.0.0.1:8765".into())
}

async fn anki(client: &reqwest::Client, action: &str, params: Value) -> Result<Value, String> {
    note_type::anki(client, &anki_url(), action, params).await
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
        println!(
            "✗ note type {} is not in this collection",
            config.model_name
        );
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
        println!(
            "    {}=<field>   # {what}, or empty for none",
            env_var_for(what)
        );
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
    println!("downloading Lapis");
    match note_type::install_lapis(client, &anki_url()).await {
        Ok(Imported::Silently) => {
            println!("✓ imported. Lapis brings its own deck; cards still go to your own.");
            check(client).await
        }
        Ok(Imported::AfterOneClick(path)) => {
            println!("→ Anki's import dialog is open on Lapis. Click Import, then:");
            println!("    anki-setup check");
            println!("  the file can be deleted afterwards: {}", path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("✗ {e}");
            ExitCode::FAILURE
        }
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
