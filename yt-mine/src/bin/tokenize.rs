use std::collections::HashSet;
use std::env;
use std::path::PathBuf;

use jp_core::tokenize::{SudachiTokenizer, Token, Tokenizer};

fn main() {
    let text: String = env::args().skip(1).collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        eprintln!("usage: tokenize <japanese text>");
        std::process::exit(1);
    }

    let dict_path: PathBuf = env::var("JP_TOOLS_SUDACHI_DICT_PATH")
        .unwrap_or_else(|_| "system_full.dic".into())
        .into();
    let tokenizer =
        SudachiTokenizer::new(&dict_path, HashSet::new()).expect("failed to initialize tokenizer");
    let tokens = tokenizer.tokenize(&text).expect("tokenization failed");

    println!("{:<14} {:<14} {:<14} {}", "surface", "base_form", "reading", "pos");
    println!("{}", "-".repeat(60));

    for Token { surface, base_form, reading, pos } in &tokens {
        println!("{:<14} {:<14} {:<14} {}", surface, base_form, reading, pos);
    }
}
