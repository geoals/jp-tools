mod html;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use serde_json::Value;
use tracing::info;

use html::structured_content_to_html;

#[derive(Debug, Clone)]
pub struct DictionaryEntry {
    pub term: String,
    pub reading: String,
    pub definitions: Vec<String>,
    pub score: i64,
}

pub struct Dictionary {
    title: String,
    entries: HashMap<String, Vec<DictionaryEntry>>,
}

#[derive(Debug, thiserror::Error)]
pub enum DictionaryError {
    #[error("failed to load dictionary: {0}")]
    Load(String),
}

impl Dictionary {
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Look up entries by exact headword match.
    pub fn lookup(&self, term: &str) -> &[DictionaryEntry] {
        self.entries.get(term).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Wrap definition HTML with dictionary title and body container divs.
    /// Class names are suffixed with a slug derived from the dictionary title,
    /// e.g. `dict-jitendex-title` / `dict-jitendex-body`.
    pub fn wrap_definitions(&self, definitions_html: &str) -> String {
        let slug = css_slug(&self.title);
        format!(
            r#"<div class="dict-{slug}-title">{title}</div><div class="dict-{slug}-body">{definitions_html}</div>"#,
            slug = slug,
            title = html::html_escape(&self.title),
            definitions_html = definitions_html,
        )
    }
}

/// Parse a single Yomitan v3 term bank entry (8-element JSON array)
/// into a `DictionaryEntry`.
fn parse_entry(arr: &[Value]) -> Option<DictionaryEntry> {
    if arr.len() < 8 {
        return None;
    }
    let term = arr[0].as_str()?.to_string();
    let reading = arr[1].as_str().unwrap_or("").to_string();
    let score = arr[4].as_i64().unwrap_or(0);
    let definitions = parse_definitions(&arr[5]);
    Some(DictionaryEntry {
        term,
        reading,
        definitions,
        score,
    })
}

/// Parse the definitions array (element 5) from a term bank entry.
/// Each element can be a string, a `{"type":"text"}` object,
/// a `{"type":"structured-content"}` object, or an image (skipped).
fn parse_definitions(value: &Value) -> Vec<String> {
    let arr = match value.as_array() {
        Some(a) => a,
        None => return vec![],
    };
    arr.iter()
        .filter_map(|def| match def {
            Value::String(s) => Some(s.clone()),
            Value::Object(obj) => {
                let type_str = obj.get("type")?.as_str()?;
                match type_str {
                    "text" => obj.get("text")?.as_str().map(|s| s.to_string()),
                    "structured-content" => {
                        let content = obj.get("content")?;
                        let html = structured_content_to_html(content);
                        if html.is_empty() { None } else { Some(html) }
                    }
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

/// Recursively extract plain text from Yomitan structured-content.
/// Handles strings, arrays, and tag objects with nested `content`.
pub(crate) fn extract_text_from_content(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr.iter().map(extract_text_from_content).collect(),
        Value::Object(obj) => {
            // Skip images
            if obj.get("tag").and_then(|t| t.as_str()) == Some("img") {
                return String::new();
            }
            // <br> -> newline
            if obj.get("tag").and_then(|t| t.as_str()) == Some("br") {
                return "\n".to_string();
            }
            if let Some(content) = obj.get("content") {
                extract_text_from_content(content)
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

/// Convert a dictionary title to a CSS-safe slug for use in class names.
/// Keeps alphanumeric and non-ASCII (e.g. CJK) characters, replaces everything
/// else with hyphens, collapses runs, strips leading/trailing hyphens, lowercases ASCII.
pub(crate) fn css_slug(title: &str) -> String {
    let mut slug = String::with_capacity(title.len());
    let mut prev_hyphen = true; // avoid leading hyphen
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_hyphen = false;
        } else if ch.is_whitespace() || ch.is_ascii_punctuation() {
            // Whitespace (including fullwidth) and ASCII punctuation → hyphen
            if !prev_hyphen {
                slug.push('-');
                prev_hyphen = true;
            }
        } else {
            // Non-ASCII content characters (CJK etc.) — keep as-is
            slug.push(ch);
            prev_hyphen = false;
        }
    }
    if slug.ends_with('-') {
        slug.pop();
    }
    slug
}

/// Parse the dictionary title from the zip's `index.json`.
/// Returns "Unknown" if the file is missing or has no `title` field.
fn parse_index_title<R: Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>) -> String {
    let mut file = match archive.by_name("index.json") {
        Ok(f) => f,
        Err(_) => return "Unknown".into(),
    };
    let mut contents = String::new();
    if file.read_to_string(&mut contents).is_err() {
        return "Unknown".into();
    }
    serde_json::from_str::<Value>(&contents)
        .ok()
        .and_then(|v| v.get("title")?.as_str().map(String::from))
        .unwrap_or_else(|| "Unknown".into())
}

/// Parse all entries from a term bank JSON string (array of arrays).
pub fn parse_term_bank(json: &str) -> Result<Vec<DictionaryEntry>, DictionaryError> {
    let value: Value =
        serde_json::from_str(json).map_err(|e| DictionaryError::Load(e.to_string()))?;
    let arr = value
        .as_array()
        .ok_or_else(|| DictionaryError::Load("term bank is not an array".into()))?;
    Ok(arr.iter().filter_map(|v| parse_entry(v.as_array()?)).collect())
}

impl Dictionary {
    /// Load a Yomitan dictionary from a zip file containing `term_bank_*.json` files.
    pub fn load_from_zip(path: &Path) -> Result<Self, DictionaryError> {
        let file = std::fs::File::open(path)
            .map_err(|e| DictionaryError::Load(format!("failed to open {}: {e}", path.display())))?;
        Self::load_from_reader(file)
    }

    /// Load a Yomitan dictionary from any reader implementing `Read + Seek`.
    /// This allows loading from files and in-memory buffers (for testing).
    fn load_from_reader<R: Read + std::io::Seek>(reader: R) -> Result<Self, DictionaryError> {
        let mut archive = zip::ZipArchive::new(reader)
            .map_err(|e| DictionaryError::Load(format!("failed to read zip: {e}")))?;

        let title = parse_index_title(&mut archive);

        let mut all_entries = Vec::new();
        let term_bank_names: Vec<String> = (0..archive.len())
            .filter_map(|i| {
                let file = archive.by_index(i).ok()?;
                let name = file.name().to_string();
                if name.contains("term_bank_") && name.ends_with(".json") {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();

        for name in &term_bank_names {
            let mut file = archive
                .by_name(name)
                .map_err(|e| DictionaryError::Load(format!("failed to read {name}: {e}")))?;
            let mut contents = String::new();
            file.read_to_string(&mut contents)
                .map_err(|e| DictionaryError::Load(format!("failed to read {name}: {e}")))?;
            let entries = parse_term_bank(&contents)?;
            all_entries.extend(entries);
        }

        info!(
            files = term_bank_names.len(),
            entries = all_entries.len(),
            "loaded dictionary"
        );
        let mut dict = Self::from_entries(all_entries);
        dict.title = title;
        Ok(dict)
    }
}

impl Dictionary {
    /// Build a dictionary from a list of parsed entries.
    /// Title defaults to "Unknown"; `load_from_reader` overrides with the zip's index.json title.
    pub fn from_entries(entries: Vec<DictionaryEntry>) -> Self {
        let mut map: HashMap<String, Vec<DictionaryEntry>> = HashMap::new();
        for entry in entries {
            map.entry(entry.term.clone()).or_default().push(entry);
        }
        // Sort each entry list by score descending (higher = more common)
        for entries in map.values_mut() {
            entries.sort_by(|a, b| b.score.cmp(&a.score));
        }
        Self {
            title: "Unknown".into(),
            entries: map,
        }
    }
}
