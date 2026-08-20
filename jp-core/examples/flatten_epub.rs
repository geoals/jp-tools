//! Flatten an epub and report what came out. `cargo run -p jp-core --example flatten_epub -- <file>`
fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: flatten_epub <file.epub>");
    let bytes = std::fs::read(&path).expect("read");
    let text = jp_core::epub::flatten(&bytes).expect("flatten");
    eprintln!(
        "{} bytes, {} counted chars, {} lines",
        text.len(),
        jp_core::text::chars::count_chars(&text),
        text.lines().count()
    );
    println!("{text}");
}
