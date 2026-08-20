//! `cargo run -p jp-core --example epub_cover -- <file.epub> <out-dir>`
fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: epub_cover <file.epub> <out-dir>");
    let out = args.next().unwrap_or_else(|| ".".into());
    let bytes = std::fs::read(&path).expect("read");
    match jp_core::epub::cover(&bytes) {
        Some(c) => {
            let dest = format!("{out}/cover.{}", c.ext);
            std::fs::write(&dest, &c.bytes).expect("write");
            println!("{dest} ({} bytes)", c.bytes.len());
        }
        None => println!("no cover found"),
    }
}
