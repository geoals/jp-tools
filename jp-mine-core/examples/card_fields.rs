//! What a mined card's fields actually come out as, against a real knowledge.db.
//!
//!   cargo run --example card_fields -p jp-mine-core -- <db> <term> [reading]
//!
//! The reading is the ledger's — `Term::new`'s, so blank for a kana headword.

use jp_mine_core::card;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    let k = jp_core::knowledge::Knowledge::open(&a[1]).await.unwrap();
    let (term, reading) = (a[2].as_str(), a.get(3).map(String::as_str).unwrap_or(""));
    let accent = card::accent(k.pool(), term, reading).await.unwrap();
    println!("furigana: {}", card::furigana(term, reading));
    println!("accent:   {accent:?}");
    println!(
        "pitchnum: {}",
        accent.map(card::pitch_num).unwrap_or_default()
    );
    println!(
        "glossary: {}",
        card::glossary(k.pool(), term, reading).await.unwrap()
    );
}
