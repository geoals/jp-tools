use std::env;

use jp_core::tokenize::{LinderaTokenizer, Token, Tokenizer};

fn main() {
    let text: String = env::args().skip(1).collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        eprintln!("usage: tokenize <japanese text>");
        std::process::exit(1);
    }

    let tokenizer = LinderaTokenizer::new().expect("failed to initialize tokenizer");
    let tokens = tokenizer.tokenize(&text).expect("tokenization failed");

    println!("{:<14} {:<14} {:<14} {}", "surface", "base_form", "reading", "pos");
    println!("{}", "-".repeat(60));

    for Token { surface, base_form, reading, pos } in &tokens {
        println!("{:<14} {:<14} {:<14} {}", surface, base_form, reading, pos);
    }
}
