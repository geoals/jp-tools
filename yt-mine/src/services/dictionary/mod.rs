mod html;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use base64::Engine;
use serde_json::Value;
use sqlx::SqlitePool;
use tracing::{info, warn};

use html::structured_content_to_html;

#[derive(Debug, Clone)]
pub struct DictionaryEntry {
    pub term: String,
    pub reading: String,
    pub definitions: Vec<String>,
    pub score: i64,
}

/// Pitch accent data for a single reading of a term.
#[derive(Debug, Clone)]
pub struct PitchEntry {
    pub reading: String,
    pub positions: Vec<u32>,
}

/// Storage backend for dictionary data.
/// Production uses `Sqlite` for lazy per-term lookups (no upfront loading).
/// Tests use `InMemory` so they don't need a database.
enum DictionaryStorage {
    InMemory {
        entries: HashMap<String, Vec<DictionaryEntry>>,
        pitch: HashMap<String, Vec<PitchEntry>>,
    },
    Sqlite {
        pool: SqlitePool,
        dict_id: i64,
    },
}

pub struct Dictionary {
    title: String,
    storage: DictionaryStorage,
}

#[derive(Debug, thiserror::Error)]
pub enum DictionaryError {
    #[error("failed to load dictionary: {0}")]
    Load(String),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

impl Dictionary {
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Look up entries by exact headword match.
    pub async fn lookup(&self, term: &str) -> Vec<DictionaryEntry> {
        match &self.storage {
            DictionaryStorage::InMemory { entries, .. } => {
                entries.get(term).cloned().unwrap_or_default()
            }
            DictionaryStorage::Sqlite { pool, dict_id } => {
                crate::db::lookup_dictionary_entries(pool, *dict_id, term)
                    .await
                    .unwrap_or_default()
            }
        }
    }

    /// Look up pitch accent entries by exact headword match.
    pub async fn lookup_pitch(&self, term: &str) -> Vec<PitchEntry> {
        match &self.storage {
            DictionaryStorage::InMemory { pitch, .. } => {
                pitch.get(term).cloned().unwrap_or_default()
            }
            DictionaryStorage::Sqlite { pool, dict_id } => {
                crate::db::lookup_pitch_entries(pool, *dict_id, term)
                    .await
                    .unwrap_or_default()
            }
        }
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

    /// Create a dictionary backed by SQLite for lazy per-term lookups.
    pub fn from_sqlite(pool: SqlitePool, dict_id: i64, title: String) -> Self {
        Self {
            title,
            storage: DictionaryStorage::Sqlite { pool, dict_id },
        }
    }
}

/// Parse a single Yomitan v3 term bank entry (8-element JSON array)
/// into a `DictionaryEntry`.
fn parse_entry(arr: &[Value], images: &HashMap<String, String>) -> Option<DictionaryEntry> {
    if arr.len() < 8 {
        return None;
    }
    let term = arr[0].as_str()?.to_string();
    let reading = arr[1].as_str().unwrap_or("").to_string();
    let score = arr[4].as_i64().unwrap_or(0);
    let definitions = parse_definitions(&arr[5], images);
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
fn parse_definitions(value: &Value, images: &HashMap<String, String>) -> Vec<String> {
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
                        let html = structured_content_to_html(content, images);
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

/// Format a term+reading pair as Anki bracket furigana notation.
/// Returns `"term[reading]"` when reading differs from term,
/// or just `"term"` when reading is empty or identical to term.
pub fn format_furigana(term: &str, reading: &str) -> String {
    if reading.is_empty() || reading == term {
        term.to_string()
    } else {
        format!("{term}[{reading}]")
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

/// Infer MIME type from file extension for image embedding.
fn mime_for_image(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".svg") {
        Some("image/svg+xml")
    } else if lower.ends_with(".png") {
        Some("image/png")
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        Some("image/jpeg")
    } else {
        None
    }
}

/// Pre-extract all image files from a zip archive into data URIs.
/// Returns a map from zip path to `data:<mime>;base64,...` string.
fn build_image_map<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> HashMap<String, String> {
    let image_names: Vec<String> = (0..archive.len())
        .filter_map(|i| {
            let file = archive.by_index(i).ok()?;
            let name = file.name().to_string();
            if mime_for_image(&name).is_some() {
                Some(name)
            } else {
                None
            }
        })
        .collect();

    let mut images = HashMap::new();
    for name in &image_names {
        let mime = match mime_for_image(name) {
            Some(m) => m,
            None => continue,
        };
        let Ok(mut file) = archive.by_name(name) else {
            continue;
        };
        let mut bytes = Vec::new();
        if file.read_to_end(&mut bytes).is_err() {
            continue;
        }
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        images.insert(name.clone(), format!("data:{mime};base64,{b64}"));
    }
    images
}

/// Parse all entries from a term bank JSON string (array of arrays).
pub fn parse_term_bank(
    json: &str,
    images: &HashMap<String, String>,
) -> Result<Vec<DictionaryEntry>, DictionaryError> {
    let value: Value =
        serde_json::from_str(json).map_err(|e| DictionaryError::Load(e.to_string()))?;
    let arr = value
        .as_array()
        .ok_or_else(|| DictionaryError::Load("term bank is not an array".into()))?;
    Ok(arr
        .iter()
        .filter_map(|v| parse_entry(v.as_array()?, images))
        .collect())
}

/// Parse pitch accent entries from a Yomitan `term_meta_bank_*.json` string.
/// Each entry is a 3-element array: `[term, "pitch", {reading, pitches: [{position}]}]`.
/// Non-pitch entries (e.g. "freq") are skipped, as are malformed entries.
pub fn parse_pitch_bank(json: &str) -> Result<Vec<(String, PitchEntry)>, DictionaryError> {
    let value: Value =
        serde_json::from_str(json).map_err(|e| DictionaryError::Load(e.to_string()))?;
    let arr = value
        .as_array()
        .ok_or_else(|| DictionaryError::Load("pitch bank is not an array".into()))?;

    Ok(arr.iter().filter_map(|v| parse_pitch_entry(v)).collect())
}

fn parse_pitch_entry(value: &Value) -> Option<(String, PitchEntry)> {
    let arr = value.as_array()?;
    if arr.len() < 3 {
        return None;
    }
    let term = arr[0].as_str()?;
    let kind = arr[1].as_str()?;
    if kind != "pitch" {
        return None;
    }
    let data = arr[2].as_object()?;
    let reading = data.get("reading")?.as_str()?.to_string();
    let pitches = data.get("pitches")?.as_array()?;
    let positions: Vec<u32> = pitches
        .iter()
        .filter_map(|p| p.get("position")?.as_u64().map(|n| n as u32))
        .collect();

    Some((
        term.to_string(),
        PitchEntry { reading, positions },
    ))
}

/// Read a zip entry as a UTF-8 string, falling back to raw decompression
/// (skipping CRC validation) if the normal read fails with "Invalid checksum".
/// Some Yomitan zips (e.g. NHK pitch accent) have incorrect CRC values.
fn read_zip_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<String, DictionaryError> {
    // Try normal read first (with CRC validation)
    {
        let mut file = archive
            .by_name(name)
            .map_err(|e| DictionaryError::Load(format!("failed to open {name}: {e}")))?;
        let mut contents = String::new();
        match file.read_to_string(&mut contents) {
            Ok(_) => return Ok(contents),
            Err(e) if e.to_string().contains("Invalid checksum") => {
                warn!(file = name, "CRC mismatch, retrying with raw decompression");
            }
            Err(e) => {
                return Err(DictionaryError::Load(format!(
                    "failed to read {name}: {e}"
                )));
            }
        }
    }

    // Fallback: read raw compressed data and decompress manually
    let index = (0..archive.len())
        .find(|&i| {
            archive
                .by_index(i)
                .ok()
                .is_some_and(|f| f.name() == name)
        })
        .ok_or_else(|| DictionaryError::Load(format!("{name} not found in archive")))?;

    let compression = archive.by_index(index).unwrap().compression();
    let mut raw = archive
        .by_index_raw(index)
        .map_err(|e| DictionaryError::Load(format!("failed to read raw {name}: {e}")))?;
    let mut compressed = Vec::new();
    raw.read_to_end(&mut compressed)
        .map_err(|e| DictionaryError::Load(format!("failed to read raw {name}: {e}")))?;

    let bytes = match compression {
        zip::CompressionMethod::Stored => compressed,
        zip::CompressionMethod::Deflated => {
            let mut decoder = flate2::read::DeflateDecoder::new(&compressed[..]);
            let mut out = Vec::new();
            decoder.read_to_end(&mut out).map_err(|e| {
                DictionaryError::Load(format!("failed to decompress {name}: {e}"))
            })?;
            out
        }
        other => {
            return Err(DictionaryError::Load(format!(
                "unsupported compression for {name}: {other}"
            )));
        }
    };

    String::from_utf8(bytes)
        .map_err(|e| DictionaryError::Load(format!("{name} is not valid UTF-8: {e}")))
}

impl Dictionary {
    /// Load a Yomitan dictionary from a zip file containing `term_bank_*.json` files.
    pub fn load_from_zip(path: &Path) -> Result<Self, DictionaryError> {
        let file = std::fs::File::open(path)
            .map_err(|e| DictionaryError::Load(format!("failed to open {}: {e}", path.display())))?;
        Self::load_from_reader(file)
    }

    /// Parse only pitch data from a zip, for backfilling existing cached dicts.
    fn load_pitch_from_zip(path: &Path) -> Result<Vec<(String, PitchEntry)>, DictionaryError> {
        let file = std::fs::File::open(path)
            .map_err(|e| DictionaryError::Load(format!("failed to open {}: {e}", path.display())))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| DictionaryError::Load(format!("failed to read zip: {e}")))?;

        let meta_bank_names: Vec<String> = (0..archive.len())
            .filter_map(|i| {
                let file = archive.by_index(i).ok()?;
                let name = file.name().to_string();
                if name.contains("term_meta_bank_") && name.ends_with(".json") {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();

        let mut all_pitch = Vec::new();
        for name in &meta_bank_names {
            let contents = read_zip_entry(&mut archive, name)?;
            let pitch_entries = parse_pitch_bank(&contents)?;
            all_pitch.extend(pitch_entries);
        }
        Ok(all_pitch)
    }

    /// Load a Yomitan dictionary from any reader implementing `Read + Seek`.
    /// This allows loading from files and in-memory buffers (for testing).
    fn load_from_reader<R: Read + std::io::Seek>(reader: R) -> Result<Self, DictionaryError> {
        let mut archive = zip::ZipArchive::new(reader)
            .map_err(|e| DictionaryError::Load(format!("failed to read zip: {e}")))?;

        let title = parse_index_title(&mut archive);

        let mut all_entries = Vec::new();
        let mut term_bank_names = Vec::new();
        let mut meta_bank_names = Vec::new();
        for i in 0..archive.len() {
            let Ok(file) = archive.by_index(i) else {
                continue;
            };
            let name = file.name().to_string();
            if name.ends_with(".json") {
                if name.contains("term_meta_bank_") {
                    meta_bank_names.push(name);
                } else if name.contains("term_bank_") {
                    term_bank_names.push(name);
                }
            }
        }

        let images = build_image_map(&mut archive);
        for name in &term_bank_names {
            let contents = read_zip_entry(&mut archive, name)?;
            let entries = parse_term_bank(&contents, &images)?;
            all_entries.extend(entries);
        }

        let mut all_pitch: Vec<(String, PitchEntry)> = Vec::new();
        for name in &meta_bank_names {
            let contents = read_zip_entry(&mut archive, name)?;
            let pitch_entries = parse_pitch_bank(&contents)?;
            all_pitch.extend(pitch_entries);
        }

        info!(
            files = term_bank_names.len(),
            entries = all_entries.len(),
            pitch_entries = all_pitch.len(),
            "loaded dictionary"
        );
        let mut dict = Self::from_entries(all_entries);
        dict.title = title;
        dict.set_pitch(all_pitch);
        Ok(dict)
    }
}

impl Dictionary {
    /// Populate pitch accent data from a list of `(term, PitchEntry)` pairs.
    /// Only valid for `InMemory`-backed dictionaries (used during zip parsing and tests).
    pub fn set_pitch(&mut self, entries: Vec<(String, PitchEntry)>) {
        let DictionaryStorage::InMemory { ref mut pitch, .. } = self.storage else {
            panic!("set_pitch called on Sqlite-backed dictionary");
        };
        let mut map: HashMap<String, Vec<PitchEntry>> = HashMap::new();
        for (term, entry) in entries {
            map.entry(term).or_default().push(entry);
        }
        *pitch = map;
    }

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
            storage: DictionaryStorage::InMemory {
                entries: map,
                pitch: HashMap::new(),
            },
        }
    }

    /// Load a dictionary from the SQLite cache, or import from the zip file if not cached.
    /// On first load the zip is parsed and entries are stored in the database.
    /// Subsequent loads return a lazy SQLite-backed handle — no data is loaded into memory.
    pub async fn load_or_import(
        pool: &sqlx::SqlitePool,
        path: &Path,
    ) -> Result<Self, DictionaryError> {
        use crate::db;

        let path_str = path.to_string_lossy();

        if let Some((id, title)) = db::find_dictionary(pool, &path_str).await? {
            // Handle existing cached dicts that were imported before pitch support:
            // if pitch table is empty but the zip might have pitch data, re-parse it.
            if !db::has_pitch_entries(pool, id).await? && path.exists() {
                if let Ok(fresh_pitch) = Self::load_pitch_from_zip(path) {
                    if !fresh_pitch.is_empty() {
                        let mut tx = pool.begin().await?;
                        db::insert_pitch_entries(&mut tx, id, &fresh_pitch).await?;
                        tx.commit().await?;
                        info!(title = %title, pitch = fresh_pitch.len(), "backfilled pitch data into cache");
                    }
                }
            }

            info!(title = %title, "loaded dictionary from cache");
            return Ok(Self::from_sqlite(pool.clone(), id, title));
        }

        // Not cached — load from zip and store in db atomically
        let dict = Self::load_from_zip(path)?;
        let DictionaryStorage::InMemory { ref entries, ref pitch } = dict.storage else {
            unreachable!("load_from_zip always creates InMemory");
        };
        let entries_vec: Vec<DictionaryEntry> =
            entries.values().flat_map(|v| v.iter()).cloned().collect();
        let pitch_vec: Vec<(String, PitchEntry)> = pitch
            .iter()
            .flat_map(|(term, entries)| {
                entries.iter().map(move |e| (term.clone(), e.clone()))
            })
            .collect();
        let dict_id = db::import_dictionary(pool, &dict.title, &path_str, &entries_vec).await?;

        if !pitch_vec.is_empty() {
            let mut tx = pool.begin().await?;
            db::insert_pitch_entries(&mut tx, dict_id, &pitch_vec).await?;
            tx.commit().await?;
        }

        info!(
            title = %dict.title,
            entries = entries_vec.len(),
            pitch = pitch_vec.len(),
            "imported dictionary into cache"
        );
        Ok(Self::from_sqlite(pool.clone(), dict_id, dict.title))
    }
}
