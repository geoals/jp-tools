//! Where the highlighter's startup time goes, step by step.
//!
//!   cargo run --release --example startup_profile -p jp-core -- \
//!     ~/.local/share/kotodex/knowledge.db system_full.dic
//!
//! Each line is one of the loads `Highlighter::build` makes, then the two whole
//! numbers to check them against. Run the same queries through the `sqlite3` CLI
//! to see how much of a step is the database and how much is decoding its rows.

use std::collections::HashSet;
use std::time::Instant;

macro_rules! timed {
    ($label:expr, $body:expr) => {{
        let t = Instant::now();
        let v = $body;
        println!("{:>8.0} ms  {}", t.elapsed().as_secs_f64() * 1000.0, $label);
        v
    }};
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = std::env::args().nth(1).expect("path to knowledge.db");
    let dict = std::env::args().nth(2).expect("path to system_full.dic");

    let whole = Instant::now();
    let k = timed!("Knowledge::open", jp_core::knowledge::Knowledge::open(&db).await?);
    let pool = k.pool();

    use jp_core::knowledge::dictionaries as d;
    let vocab = timed!("vocabulary::mined_vocab", jp_core::knowledge::vocabulary::mined_vocab(pool).await?);
    let lexicon = timed!("master_headwords", d::master_headwords(pool).await?);
    let readings = timed!("master_entries", d::master_entries(pool).await?);
    let conjugatable = timed!("master_conjugatable", d::master_conjugatable(pool).await?);
    let standard = timed!("standard_entries", d::standard_entries(pool).await?);
    let names: HashSet<String> = timed!(
        "work_names::all",
        jp_core::knowledge::work_names::all(&k).await?.into_iter().collect()
    );
    let (listed, listed_readings) = timed!("wordhood_entries", d::wordhood_entries(pool).await?);
    println!(
        "         rows: vocab={} lexicon={} readings={} conjugatable={} standard={} names={} listed={}/{}",
        vocab.len(),
        lexicon.len(),
        readings.len(),
        conjugatable.len(),
        standard.len(),
        names.len(),
        listed.len(),
        listed_readings.len()
    );

    // The three private loads inside pipeline(), by the same SQL they run.
    let jiten = d::reader_frequency(pool).await?.unwrap();
    let bccwj = d::by_title(pool, "BCCWJ").await?.unwrap();
    let master = d::master(pool).await?.unwrap();
    let jitendex = d::by_title(pool, "Jitendex").await?.unwrap();
    println!("         jiten id={} bccwj id={}", jiten.id, bccwj.id);
    let jrows = timed!("reader_ranks (Jiten GROUP BY)", {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT term, MIN(frequency) FROM dictionary_frequency WHERE dictionary_id = ? GROUP BY term",
        )
        .bind(jiten.id)
        .fetch_all(pool)
        .await?;
        rows.len()
    });
    let brows = timed!("BCCWJ GROUP BY (reader_ranks_for)", {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT term, MIN(frequency) FROM dictionary_frequency WHERE dictionary_id = ? GROUP BY term",
        )
        .bind(bccwj.id)
        .fetch_all(pool)
        .await?;
        rows.len()
    });
    let ambiguous = jp_core::tokenize::ambiguous_headwords(&readings);
    println!("         jiten rows={jrows} bccwj rows={brows} ambiguous={}", ambiguous.len());
    let amb = timed!("ambiguous_ranks (chunked IN)", d::frequency_ranks(pool, bccwj.id, &ambiguous).await?);
    println!("         ambiguous ranks={}", amb.len());
    let pref = timed!("preferred_readings", d::preferred_readings(pool, master.id, jitendex.id, bccwj.id).await?);
    println!("         preferred={}", pref.len());

    let tok = timed!("SudachiTokenizer::new", jp_core::tokenize::SudachiTokenizer::new(std::path::Path::new(&dict), vocab.clone())?);
    drop(tok);

    let p = timed!("pipeline() whole", jp_core::highlight::pipeline(&k, &dict).await?);
    drop(p);
    let h = timed!("Highlighter::build() whole", jp_core::highlight::Highlighter::build(&k, &dict).await?);
    drop(h);
    println!("{:>8.0} ms  TOTAL", whole.elapsed().as_secs_f64() * 1000.0);
    Ok(())
}
