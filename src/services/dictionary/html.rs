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
pub(crate) fn render_style(obj: &serde_json::Map<String, Value>) -> String {
    let mut props: Vec<(String, &str)> = obj
        .iter()
        .filter_map(|(k, v)| {
            let val = v.as_str()?;
            Some((camel_to_kebab(k), val))
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
pub(crate) fn structured_content_to_html(value: &Value) -> String {
    let mut buf = String::new();
    write_html(value, &mut buf);
    buf
}

fn write_html(value: &Value, buf: &mut String) {
    match value {
        Value::String(s) => buf.push_str(&html_escape(s)),
        Value::Array(arr) => {
            for child in arr {
                write_html(child, buf);
            }
        }
        Value::Object(obj) => write_html_object(obj, buf),
        _ => {}
    }
}

fn write_html_object(obj: &serde_json::Map<String, Value>, buf: &mut String) {
    let tag = match obj.get("tag").and_then(|t| t.as_str()) {
        Some("img") => return,
        Some("br") => {
            buf.push_str("<br>");
            return;
        }
        Some(t) => t,
        None => {
            // No tag — just recurse into content
            if let Some(content) = obj.get("content") {
                write_html(content, buf);
            }
            return;
        }
    };

    buf.push('<');
    buf.push_str(tag);
    write_attributes(obj, buf);
    buf.push('>');

    if let Some(content) = obj.get("content") {
        write_html(content, buf);
    }

    buf.push_str("</");
    buf.push_str(tag);
    buf.push('>');
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
