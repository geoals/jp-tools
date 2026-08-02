//! Dump every known form with the lexeme it collapses onto, TSV.
//!
//! The grouping behind "I know N words", as data — for ranking words by
//! frequency, or for seeing which spellings fold into which.
//!
//! ```text
//! cargo run --example lexemes -p jp-core -- <knowledge.db>
//! ```

use jp_core::knowledge::Knowledge;
use jp_core::knowledge::lexeme::{self, Lexeme};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let k = Knowledge::open(&args[1]).await.unwrap();

    for (term, lexeme) in lexeme::resolved_forms(&k).await.unwrap() {
        let id = match lexeme {
            Lexeme::Seq(s) => format!("S{s}"),
            Lexeme::Form(h, r) => format!("F{h}|{r}"),
        };
        println!("{}\t{}\t{}", term.headword, term.reading, id);
    }
}
