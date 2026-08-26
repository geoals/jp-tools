//! The Japanese-capable fonts installed on this machine.
//!
//! The overlay's font list used to be eight names hardcoded in the page, which
//! on any machine but this one is a row of chips that do nothing. `fc-list`
//! answers it properly, and only the server can ask: the page is a browser tab.

use axum::Json;
use serde_json::{Value, json};
use std::process::Command;

/// `GET /api/reader/fonts` — Japanese-capable font families, sorted, unique.
///
/// No fontconfig, or none installed: an empty list, never an error. The panel
/// then offers the font the shell was launched with and nothing else, which is
/// what it does anyway before the answer arrives.
pub async fn fonts() -> Json<Value> {
    Json(json!({ "families": families() }))
}

fn families() -> Vec<String> {
    let Ok(out) = Command::new("fc-list")
        .args([":lang=ja", "family"])
        .output()
    else {
        return Vec::new();
    };
    let mut names: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        // One line can carry several names for the same family, comma
        // separated — the localized one among them. The first is the name a
        // CSS `font-family` is written with.
        .filter_map(|line| line.split(',').next())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}
