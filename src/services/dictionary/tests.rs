use super::*;
use super::html::{html_escape, camel_to_kebab, render_style, structured_content_to_html};

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
    assert_eq!(parse_definitions(&v), vec!["<span>structured def</span>"]);
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
    // structured-content "sc text" is a plain JSON string, so it gets HTML-escaped
    // (no change for simple text), but goes through structured_content_to_html
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

// --- css_slug ---

#[test]
fn css_slug_simple_name() {
    assert_eq!(css_slug("Jitendex"), "jitendex");
}

#[test]
fn css_slug_strips_non_alphanumeric() {
    assert_eq!(css_slug("Jitendex.org [2024-12-29]"), "jitendex-org-2024-12-29");
}

#[test]
fn css_slug_japanese_dictionary() {
    assert_eq!(css_slug("三省堂国語辞典　第八版"), "三省堂国語辞典-第八版");
}

#[test]
fn css_slug_collapses_consecutive_hyphens() {
    assert_eq!(css_slug("a -- b"), "a-b");
}

// --- wrap_definitions ---

#[test]
fn wrap_definitions_produces_title_and_body() {
    let dict = Dictionary::from_entries(vec![]);
    let html = dict.wrap_definitions("some definition");
    assert_eq!(
        html,
        r#"<div class="dict-unknown-title">Unknown</div><div class="dict-unknown-body">some definition</div>"#
    );
}

// --- title ---

#[test]
fn load_from_zip_parses_title_from_index_json() {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default();

        zip.start_file("index.json", options).unwrap();
        std::io::Write::write_all(
            &mut zip,
            r#"{"title": "Jitendex.org [2024-12-29]", "revision": "1"}"#.as_bytes(),
        )
        .unwrap();

        zip.start_file("term_bank_1.json", options).unwrap();
        std::io::Write::write_all(
            &mut zip,
            r#"[["食べる", "たべる", "", "v1", 100, ["to eat"], 1, ""]]"#.as_bytes(),
        )
        .unwrap();

        zip.finish().unwrap();
    }
    buf.set_position(0);

    let dict = Dictionary::load_from_reader(buf).unwrap();
    assert_eq!(dict.title(), "Jitendex.org [2024-12-29]");
}

#[test]
fn load_from_zip_missing_title_uses_unknown() {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default();

        zip.start_file("index.json", options).unwrap();
        std::io::Write::write_all(&mut zip, b"{}").unwrap();

        zip.start_file("term_bank_1.json", options).unwrap();
        std::io::Write::write_all(
            &mut zip,
            r#"[["食べる", "たべる", "", "v1", 100, ["to eat"], 1, ""]]"#.as_bytes(),
        )
        .unwrap();

        zip.finish().unwrap();
    }
    buf.set_position(0);

    let dict = Dictionary::load_from_reader(buf).unwrap();
    assert_eq!(dict.title(), "Unknown");
}

// --- html_escape ---

#[test]
fn html_escape_passes_through_plain_text() {
    assert_eq!(html_escape("hello world"), "hello world");
}

#[test]
fn html_escape_escapes_ampersand() {
    assert_eq!(html_escape("a & b"), "a &amp; b");
}

#[test]
fn html_escape_escapes_angle_brackets() {
    assert_eq!(html_escape("<script>"), "&lt;script&gt;");
}

#[test]
fn html_escape_escapes_quotes() {
    assert_eq!(html_escape(r#"it's "fine""#), "it&#x27;s &quot;fine&quot;");
}

// --- camel_to_kebab ---

#[test]
fn camel_to_kebab_single_word() {
    assert_eq!(camel_to_kebab("font"), "font");
}

#[test]
fn camel_to_kebab_two_words() {
    assert_eq!(camel_to_kebab("fontSize"), "font-size");
}

#[test]
fn camel_to_kebab_multiple_words() {
    assert_eq!(camel_to_kebab("borderTopWidth"), "border-top-width");
}

// --- render_style ---

#[test]
fn render_style_single_property() {
    let obj: serde_json::Map<String, Value> =
        serde_json::from_str(r#"{"fontWeight": "bold"}"#).unwrap();
    assert_eq!(render_style(&obj), "font-weight:bold");
}

#[test]
fn render_style_multiple_properties_sorted() {
    let obj: serde_json::Map<String, Value> =
        serde_json::from_str(r#"{"fontSize": "1em", "color": "red"}"#).unwrap();
    // Sorted alphabetically by CSS property name
    assert_eq!(render_style(&obj), "color:red;font-size:1em");
}

// --- structured_content_to_html ---

#[test]
fn sc_html_plain_string() {
    let v = serde_json::json!("hello");
    assert_eq!(structured_content_to_html(&v), "hello");
}

#[test]
fn sc_html_string_is_escaped() {
    let v = serde_json::json!("a < b & c > d");
    assert_eq!(structured_content_to_html(&v), "a &lt; b &amp; c &gt; d");
}

#[test]
fn sc_html_array_concatenates_children() {
    let v = serde_json::json!(["hello", " ", "world"]);
    assert_eq!(structured_content_to_html(&v), "hello world");
}

#[test]
fn sc_html_br_tag() {
    let v = serde_json::json!({"tag": "br"});
    assert_eq!(structured_content_to_html(&v), "<br>");
}

#[test]
fn sc_html_img_tag_skipped() {
    let v = serde_json::json!({"tag": "img", "path": "image.png"});
    assert_eq!(structured_content_to_html(&v), "");
}

#[test]
fn sc_html_simple_span() {
    let v = serde_json::json!({"tag": "span", "content": "text"});
    assert_eq!(structured_content_to_html(&v), "<span>text</span>");
}

#[test]
fn sc_html_nested_tags() {
    let v = serde_json::json!({
        "tag": "div",
        "content": [
            {"tag": "span", "content": "A"},
            {"tag": "br"},
            {"tag": "span", "content": "B"}
        ]
    });
    assert_eq!(
        structured_content_to_html(&v),
        "<div><span>A</span><br><span>B</span></div>"
    );
}

#[test]
fn sc_html_tag_with_style() {
    let v = serde_json::json!({
        "tag": "span",
        "style": {"fontWeight": "bold"},
        "content": "text"
    });
    assert_eq!(
        structured_content_to_html(&v),
        r#"<span style="font-weight:bold">text</span>"#
    );
}

#[test]
fn sc_html_tag_with_lang() {
    let v = serde_json::json!({
        "tag": "span",
        "lang": "ja",
        "content": "日本語"
    });
    assert_eq!(
        structured_content_to_html(&v),
        r#"<span lang="ja">日本語</span>"#
    );
}

#[test]
fn sc_html_tag_with_href() {
    let v = serde_json::json!({
        "tag": "a",
        "href": "https://example.com",
        "content": "link"
    });
    assert_eq!(
        structured_content_to_html(&v),
        r#"<a href="https://example.com">link</a>"#
    );
}

#[test]
fn sc_html_tag_with_data_attributes() {
    let v = serde_json::json!({
        "tag": "span",
        "data": {"wordId": "123", "category": "verb"},
        "content": "text"
    });
    assert_eq!(
        structured_content_to_html(&v),
        r#"<span data-category="verb" data-wordId="123">text</span>"#
    );
}

#[test]
fn sc_html_attribute_order() {
    // lang, title, href, data-*, style
    let v = serde_json::json!({
        "tag": "a",
        "style": {"color": "red"},
        "href": "https://example.com",
        "lang": "ja",
        "title": "tooltip",
        "data": {"x": "1"},
        "content": "text"
    });
    assert_eq!(
        structured_content_to_html(&v),
        r#"<a lang="ja" title="tooltip" href="https://example.com" data-x="1" style="color:red">text</a>"#
    );
}

#[test]
fn sc_html_object_without_tag_recurses_content() {
    let v = serde_json::json!({"content": "just text"});
    assert_eq!(structured_content_to_html(&v), "just text");
}

#[test]
fn sc_html_empty_tag_no_content() {
    let v = serde_json::json!({"tag": "span"});
    assert_eq!(structured_content_to_html(&v), "<span></span>");
}

#[test]
fn sc_html_ruby_text() {
    // Common Yomitan pattern for furigana
    let v = serde_json::json!({
        "tag": "ruby",
        "content": [
            "漢字",
            {"tag": "rp", "content": "("},
            {"tag": "rt", "content": "かんじ"},
            {"tag": "rp", "content": ")"}
        ]
    });
    assert_eq!(
        structured_content_to_html(&v),
        "<ruby>漢字<rp>(</rp><rt>かんじ</rt><rp>)</rp></ruby>"
    );
}

#[test]
fn sc_html_list_structure() {
    let v = serde_json::json!({
        "tag": "ul",
        "content": [
            {"tag": "li", "content": "first"},
            {"tag": "li", "content": "second"}
        ]
    });
    assert_eq!(
        structured_content_to_html(&v),
        "<ul><li>first</li><li>second</li></ul>"
    );
}
