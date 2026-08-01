//! Dump the reading preferences the tokenizer gets, through the query that
//! feeds it — the tool for re-tuning `POPULARITY_TIER` against real data, and
//! the input to `probe --prefer`.
//!
//! Pass a frequency zip to re-import it first if its rows predate the `reading`
//! column, since the tie-break needs readings.
//!
//! ```text
//! cargo run --example preferences -p jp-core -- <knowledge.db> [bccwj.zip]
//! ```

use std::path::Path;

use jp_core::knowledge::Knowledge;
use jp_core::knowledge::dictionaries;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let k = Knowledge::open(&args[1]).await.unwrap();
    let pool = k.pool();

    let named = async |title: &str| {
        dictionaries::by_title(pool, title)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{title} is not loaded"))
    };
    let bccwj = named("BCCWJ").await;

    if let Some(zip) = args.get(2) {
        let with_readings: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM dictionary_frequency WHERE dictionary_id = ? AND reading <> ''",
        )
        .bind(bccwj.id)
        .fetch_one(pool)
        .await
        .unwrap();
        if with_readings.0 == 0 {
            eprintln!("re-importing {zip} for its readings…");
            sqlx::query("DELETE FROM dictionary_frequency WHERE dictionary_id = ?")
                .bind(bccwj.id)
                .execute(pool)
                .await
                .unwrap();
            jp_core::dictionary::Dictionary::load_or_import(pool, Path::new(zip))
                .await
                .unwrap();
        }
    }

    let master = dictionaries::master(pool).await.unwrap().expect("no master");
    let jitendex = named("Jitendex").await;
    let prefs = dictionaries::preferred_readings(pool, master.id, jitendex.id, bccwj.id)
        .await
        .unwrap();
    eprintln!("{} headwords get a preferred reading", prefs.len());

    let mut rows: Vec<_> = prefs.into_iter().collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    for (term, p) in rows {
        let mut acceptable: Vec<_> = p.acceptable.into_iter().collect();
        acceptable.sort();
        println!("{term}\t{}\t{}", p.preferred, acceptable.join(","));
    }
}
