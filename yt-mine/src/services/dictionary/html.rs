use std::collections::HashMap;

use serde_json::Value;

/// Escape special HTML characters in text content.
pub(crate) fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Convert a camelCase CSS property name to kebab-case.
/// e.g. `fontSize` → `font-size`
pub(crate) fn camel_to_kebab(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for ch in s.chars() {
        if ch.is_ascii_uppercase() {
            out.push('-');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Render a JSON style object to a CSS inline style string.
/// Properties are sorted alphabetically by kebab-case name for deterministic output.
/// Style prefixes stripped from inline styles — our stylesheet controls layout and theming.
/// Yomitan styles target light-background popups and conflict with our dark theme.
const STRIPPED_STYLE_PROPS: &[&str] = &[
    "padding", "margin", "background", "border", "border-radius",
];

pub(crate) fn render_style(obj: &serde_json::Map<String, Value>) -> String {
    let mut props: Vec<(String, &str)> = obj
        .iter()
        .filter_map(|(k, v)| {
            let val = v.as_str()?;
            let kebab = camel_to_kebab(k);
            if STRIPPED_STYLE_PROPS.iter().any(|p| kebab.starts_with(p)) {
                return None;
            }
            Some((kebab, val))
        })
        .collect();
    props.sort_by(|a, b| a.0.cmp(&b.0));
    props
        .iter()
        .map(|(k, v)| format!("{k}:{v}"))
        .collect::<Vec<_>>()
        .join(";")
}

/// Convert Yomitan structured-content JSON to an HTML string.
/// The `images` map provides pre-extracted image data URIs keyed by zip path,
/// used to embed `img` tags inline rather than skipping them.
pub(crate) fn structured_content_to_html(
    value: &Value,
    images: &HashMap<String, String>,
) -> String {
    let mut writer = HtmlWriter {
        buf: String::new(),
        images,
    };
    writer.write(value);
    writer.buf
}

/// Recursive HTML writer that carries an image map for embedding `img` tags.
struct HtmlWriter<'a> {
    buf: String,
    images: &'a HashMap<String, String>,
}

impl HtmlWriter<'_> {
    fn write(&mut self, value: &Value) {
        match value {
            Value::String(s) => self.buf.push_str(&html_escape(s)),
            Value::Array(arr) => {
                for child in arr {
                    self.write(child);
                }
            }
            Value::Object(obj) => self.write_object(obj),
            _ => {}
        }
    }

    fn write_object(&mut self, obj: &serde_json::Map<String, Value>) {
        let tag = match obj.get("tag").and_then(|t| t.as_str()) {
            Some("img") => return self.write_img(obj),
            Some("br") => {
                self.buf.push_str("<br>");
                return;
            }
            Some(t) => t,
            None => {
                // No tag — just recurse into content
                if let Some(content) = obj.get("content") {
                    self.write(content);
                }
                return;
            }
        };

        self.buf.push('<');
        self.buf.push_str(tag);
        write_attributes(obj, &mut self.buf);
        self.buf.push('>');

        if let Some(content) = obj.get("content") {
            self.write(content);
        }

        self.buf.push_str("</");
        self.buf.push_str(tag);
        self.buf.push('>');
    }

    /// Render an `img` tag. If the image path exists in the pre-extracted image map,
    /// emit an `<img>` with an inline data URI. Otherwise skip (same as before).
    fn write_img(&mut self, obj: &serde_json::Map<String, Value>) {
        let path = match obj.get("path").and_then(|p| p.as_str()) {
            Some(p) => p,
            None => return,
        };
        let data_uri = match self.images.get(path) {
            Some(uri) => uri,
            None => return,
        };

        self.buf.push_str("<img src=\"");
        self.buf.push_str(data_uri);
        self.buf.push('"');

        // Build style from width/height/sizeUnits
        let units = obj
            .get("sizeUnits")
            .and_then(|v| v.as_str())
            .unwrap_or("em");
        let width = obj.get("width").and_then(|v| v.as_f64());
        let height = obj.get("height").and_then(|v| v.as_f64());

        if width.is_some() || height.is_some() {
            self.buf.push_str(" style=\"");
            let mut first = true;
            if let Some(w) = width {
                write_dimension(&mut self.buf, "width", w, units);
                first = false;
            }
            if let Some(h) = height {
                if !first {
                    self.buf.push(';');
                }
                write_dimension(&mut self.buf, "height", h, units);
            }
            self.buf.push('"');
        }

        self.buf.push('>');
    }
}

/// Format a CSS dimension value, omitting the decimal point for whole numbers.
fn write_dimension(buf: &mut String, prop: &str, value: f64, units: &str) {
    buf.push_str(prop);
    buf.push(':');
    if value.fract() == 0.0 {
        // Write as integer to avoid "1.0em" → "1em"
        buf.push_str(&format!("{}", value as i64));
    } else {
        buf.push_str(&format!("{value}"));
    }
    buf.push_str(units);
}

/// Write HTML attributes in deterministic order: lang, title, href, data-* (sorted), style.
fn write_attributes(obj: &serde_json::Map<String, Value>, buf: &mut String) {
    // lang
    if let Some(Value::String(val)) = obj.get("lang") {
        write_attr(buf, "lang", val);
    }

    // title
    if let Some(Value::String(val)) = obj.get("title") {
        write_attr(buf, "title", val);
    }

    // href
    if let Some(Value::String(val)) = obj.get("href") {
        write_attr(buf, "href", val);
    }

    // data-* attributes (sorted alphabetically by key)
    if let Some(Value::Object(data)) = obj.get("data") {
        let mut keys: Vec<&String> = data.keys().collect();
        keys.sort();
        for key in keys {
            if let Some(Value::String(val)) = data.get(key) {
                write_attr(buf, &format!("data-{key}"), val);
            }
        }
    }

    // style
    if let Some(Value::Object(style)) = obj.get("style") {
        let css = render_style(style);
        if !css.is_empty() {
            write_attr(buf, "style", &css);
        }
    }
}

fn write_attr(buf: &mut String, name: &str, value: &str) {
    buf.push(' ');
    buf.push_str(name);
    buf.push_str("=\"");
    buf.push_str(&html_escape(value));
    buf.push('"');
}
