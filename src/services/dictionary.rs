use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use serde_json::Value;
use tracing::info;

#[derive(Debug, Clone)]
pub struct DictionaryEntry {
    pub term: String,
    pub reading: String,
    pub definitions: Vec<String>,
    pub score: i64,
}

pub struct Dictionary {
    entries: HashMap<String, Vec<DictionaryEntry>>,
}

#[derive(Debug, thiserror::Error)]
pub enum DictionaryError {
    #[error("failed to load dictionary: {0}")]
    Load(String),
}

impl Dictionary {
    /// Look up entries by exact headword match.
    pub fn lookup(&self, term: &str) -> &[DictionaryEntry] {
        self.entries.get(term).map(|v| v.as_slice()).unwrap_or(&[])
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
                        let text = extract_text_from_content(content);
                        if text.is_empty() { None } else { Some(text) }
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
pub fn extract_text_from_content(value: &Value) -> String {
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
        Ok(Self::from_entries(all_entries))
    }
}

impl Dictionary {
    /// Build a dictionary from a list of parsed entries.
    pub fn from_entries(entries: Vec<DictionaryEntry>) -> Self {
        let mut map: HashMap<String, Vec<DictionaryEntry>> = HashMap::new();
        for entry in entries {
            map.entry(entry.term.clone()).or_default().push(entry);
        }
        // Sort each entry list by score descending (higher = more common)
        for entries in map.values_mut() {
            entries.sort_by(|a, b| b.score.cmp(&a.score));
        }
        Self { entries: map }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_text_from_plain_string() {
        let v = Value::String("hello".into());
        assert_eq!(extract_text_from_content(&v), "hello");
    }

    #[test]
    fn extract_text_from_array() {
        let v = serde_json::json!(["hello", " ", "world"]);
        assert_eq!(extract_text_from_content(&v), "hello world");
    }

    #[test]
    fn extract_text_from_tag_object() {
        let v = serde_json::json!({"tag": "span", "content": "inside span"});
        assert_eq!(extract_text_from_content(&v), "inside span");
    }

    #[test]
    fn extract_text_from_nested_structure() {
        let v = serde_json::json!({
            "tag": "div",
            "content": [
                {"tag": "span", "content": "A"},
                {"tag": "br"},
                {"tag": "span", "content": "B"}
            ]
        });
        assert_eq!(extract_text_from_content(&v), "A\nB");
    }

    #[test]
    fn extract_text_skips_images() {
        let v = serde_json::json!({
            "tag": "div",
            "content": [
                "text",
                {"tag": "img", "path": "img.png"}
            ]
        });
        assert_eq!(extract_text_from_content(&v), "text");
    }

    #[test]
    fn parse_definitions_string_entries() {
        let v = serde_json::json!(["def1", "def2"]);
        assert_eq!(parse_definitions(&v), vec!["def1", "def2"]);
    }

    #[test]
    fn parse_definitions_text_type() {
        let v = serde_json::json!([{"type": "text", "text": "a definition"}]);
        assert_eq!(parse_definitions(&v), vec!["a definition"]);
    }

    #[test]
    fn parse_definitions_structured_content() {
        let v = serde_json::json!([{
            "type": "structured-content",
            "content": {"tag": "span", "content": "structured def"}
        }]);
        assert_eq!(parse_definitions(&v), vec!["structured def"]);
    }

    #[test]
    fn parse_definitions_mixed() {
        let v = serde_json::json!([
            "simple",
            {"type": "text", "text": "text type"},
            {"type": "structured-content", "content": "sc text"},
            {"type": "image", "path": "img.png"}
        ]);
        let defs = parse_definitions(&v);
        assert_eq!(defs, vec!["simple", "text type", "sc text"]);
    }

    #[test]
    fn parse_entry_from_8_element_array() {
        let json = r#"["食べる", "たべる", "", "v1", 100, ["to eat", "to consume"], 1, ""]"#;
        let arr: Vec<Value> = serde_json::from_str(json).unwrap();
        let entry = parse_entry(&arr).unwrap();
        assert_eq!(entry.term, "食べる");
        assert_eq!(entry.reading, "たべる");
        assert_eq!(entry.score, 100);
        assert_eq!(entry.definitions, vec!["to eat", "to consume"]);
    }

    #[test]
    fn parse_entry_rejects_short_array() {
        let arr: Vec<Value> = serde_json::from_str(r#"["食べる", "たべる"]"#).unwrap();
        assert!(parse_entry(&arr).is_none());
    }

    #[test]
    fn parse_term_bank_multiple_entries() {
        let json = r#"[
            ["食べる", "たべる", "", "v1", 100, ["to eat"], 1, ""],
            ["飲む", "のむ", "", "v5", 80, ["to drink"], 2, ""]
        ]"#;
        let entries = parse_term_bank(json).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].term, "食べる");
        assert_eq!(entries[1].term, "飲む");
    }

    #[test]
    fn dictionary_lookup_exact_match() {
        let entries = vec![
            DictionaryEntry {
                term: "食べる".into(),
                reading: "たべる".into(),
                definitions: vec!["to eat".into()],
                score: 100,
            },
            DictionaryEntry {
                term: "食べる".into(),
                reading: "たべる".into(),
                definitions: vec!["to consume".into()],
                score: 50,
            },
        ];
        let dict = Dictionary::from_entries(entries);
        let results = dict.lookup("食べる");
        assert_eq!(results.len(), 2);
        // Sorted by score descending
        assert_eq!(results[0].score, 100);
        assert_eq!(results[1].score, 50);
    }

    #[test]
    fn dictionary_lookup_no_match() {
        let dict = Dictionary::from_entries(vec![]);
        assert!(dict.lookup("missing").is_empty());
    }

    #[test]
    fn load_from_zip_in_memory() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("term_bank_1.json", options).unwrap();
            let json = r#"[
                ["食べる", "たべる", "", "v1", 100, ["to eat"], 1, ""],
                ["飲む", "のむ", "", "v5", 80, ["to drink"], 2, ""]
            ]"#;
            std::io::Write::write_all(&mut zip, json.as_bytes()).unwrap();

            // Add a non-term-bank file that should be ignored
            zip.start_file("index.json", options).unwrap();
            std::io::Write::write_all(&mut zip, b"{}").unwrap();

            zip.finish().unwrap();
        }
        buf.set_position(0);

        let dict = Dictionary::load_from_reader(buf).unwrap();
        assert_eq!(dict.lookup("食べる").len(), 1);
        assert_eq!(dict.lookup("食べる")[0].definitions, vec!["to eat"]);
        assert_eq!(dict.lookup("飲む").len(), 1);
        assert!(dict.lookup("missing").is_empty());
    }

    #[test]
    fn load_from_zip_multiple_term_banks() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let options = zip::write::SimpleFileOptions::default();

            zip.start_file("term_bank_1.json", options).unwrap();
            std::io::Write::write_all(
                &mut zip,
                r#"[["食べる", "たべる", "", "v1", 100, ["to eat"], 1, ""]]"#.as_bytes(),
            )
            .unwrap();

            zip.start_file("term_bank_2.json", options).unwrap();
            std::io::Write::write_all(
                &mut zip,
                r#"[["飲む", "のむ", "", "v5", 80, ["to drink"], 2, ""]]"#.as_bytes(),
            )
            .unwrap();

            zip.finish().unwrap();
        }
        buf.set_position(0);

        let dict = Dictionary::load_from_reader(buf).unwrap();
        assert_eq!(dict.lookup("食べる").len(), 1);
        assert_eq!(dict.lookup("飲む").len(), 1);
    }
}
