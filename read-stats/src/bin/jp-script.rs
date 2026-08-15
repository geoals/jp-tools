//! Profile a work's script: what words it contains, and how many are known.
//!
//! Deriving this is a batch job over shared state, so it is a binary rather
//! than an endpoint — the same reasoning that keeps zip importing in `jp-dict`.
//! It lives in read-stats' crate because the wordhood gate and the
//! name-majority rule it has to reuse are `ingest`'s, and a profile counted by
//! a second implementation of those could not be compared with the ledger.
//!
//! Input is plain text, one line per line of script, so any source works —
//! `vn-mine/cs2-script.py`, a texthooker dump, a book. Getting the text out of
//! a game is the extractor's job, not this one's.
//!
//!     jp-script profile <work> <file.txt>    derive and store the profile
//!     jp-script names <work> [vndb-id]       import the cast, so the tokenizer
//!                                            stops splitting it
//!     jp-script show <work>                  coverage and the top unknown words

use std::path::Path;

use jp_core::knowledge::{Knowledge, work_names, work_scripts};
use read_stats::config::Config;
use read_stats::services::vndb;

const USAGE: &str = "\
jp-script — what a work's script contains, against what is known

usage:
  jp-script profile <work> <file.txt>   tokenize a script and store its profile
  jp-script names <work> [vndb-id]      import the cast from VNDB, so the
                                        tokenizer stops splitting names
  jp-script names <work> add <name>...  names VNDB does not list — a nickname,
                                        a minor character, a place. Kept
                                        separately, so a refetch cannot drop
                                        them
  jp-script names <work> list           what the tokenizer will be told
  jp-script show <work> [limit]         coverage, and the unknown words it
                                        leans on hardest (default limit 30)

<work> is the exact title `lines.work` uses, so the profile joins to
everything else recorded about that work.
";

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("jp-script: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = Config::from_env();
    let knowledge = Knowledge::open(&cfg.knowledge_db_path).await?;

    match args.first().map(String::as_str) {
        Some("profile") => {
            let (Some(work), Some(file)) = (args.get(1), args.get(2)) else {
                return Err(format!("profile needs a work and a file\n\n{USAGE}").into());
            };
            profile(&knowledge, &cfg.sudachi_dict_path, work, Path::new(file)).await
        }
        Some("names") => {
            let Some(work) = args.get(1) else {
                return Err(format!("names needs a work\n\n{USAGE}").into());
            };
            match args.get(2).map(String::as_str) {
                Some("add") => add_names(&knowledge, work, &args[3..]).await,
                Some("list") => list_names(&knowledge, work).await,
                vndb_id => names(&knowledge, work, vndb_id).await,
            }
        }
        Some("show") => {
            let Some(work) = args.get(1) else {
                return Err(format!("show needs a work\n\n{USAGE}").into());
            };
            let limit = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
            show(&knowledge, work, limit).await
        }
        _ => {
            print!("{USAGE}");
            Ok(())
        }
    }
}

async fn profile(
    knowledge: &Knowledge,
    dict_path: &Path,
    work: &str,
    file: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(file)?;
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    println!("{}: {} lines, tokenizing...", work, lines.len());

    let (encounters, total) =
        read_stats::ingest::profile_script(knowledge, dict_path, work, lines).await?;
    work_scripts::record_script(knowledge, work, total, &encounters).await?;

    println!("{} distinct terms, {total} occurrences", encounters.len());
    report(knowledge, work).await
}

/// Import a work's cast from VNDB into `work_names`.
///
/// The id is looked up from the title when not given. A wrong match is worse
/// than none — it would teach the tokenizer another VN's cast — so the id it
/// resolved is printed with the names.
async fn names(
    knowledge: &Knowledge,
    work: &str,
    vndb_id: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::builder()
        .user_agent("jp-tools/0.1")
        .build()?;
    let id = match vndb_id {
        Some(given) => {
            vndb::normalize_id(given).ok_or_else(|| format!("not a vndb id: {given}"))?
        }
        None => vndb::find_vn_id(&client, work)
            .await?
            .ok_or_else(|| format!("no VNDB match for {work} — pass the id"))?,
    };
    let cast = vndb::fetch_cast(&client, &id).await?;
    if cast.is_empty() {
        return Err(format!("{id} lists no characters").into());
    }
    let written = work_names::replace(knowledge, work, "vndb", &cast).await?;
    println!("{work} — {id}: {written} names");
    println!("  {}", cast.join("  "));
    println!();
    println!("The tokenizer reads these on its next build; re-ingest to re-derive");
    println!("what has already been counted.");
    Ok(())
}

/// Add names by hand, under their own source so a VNDB refetch keeps them.
///
/// VNDB lists the cast a player would name — it has ウィリアム・シェイクスピア
/// and not the ウィル everyone in the script calls him, and no walk-on part at
/// all. Those are exactly the names the tokenizer splits.
async fn add_names(
    knowledge: &Knowledge,
    work: &str,
    names: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    if names.is_empty() {
        return Err(format!("add needs at least one name\n\n{USAGE}").into());
    }
    // `replace` writes one source's whole set, so read what is there, add to
    // it, and put it back.
    let mut kept: Vec<String> =
        sqlx::query_scalar("SELECT name FROM work_names WHERE work = ? AND source = 'manual'")
            .bind(work)
            .fetch_all(knowledge.pool())
            .await?;
    for name in names {
        if !kept.contains(name) {
            kept.push(name.clone());
        }
    }
    work_names::replace(knowledge, work, "manual", &kept).await?;
    println!("{work}: {} names added by hand", kept.len());
    list_names(knowledge, work).await
}

async fn list_names(knowledge: &Knowledge, work: &str) -> Result<(), Box<dyn std::error::Error>> {
    let names = work_names::of_work(knowledge, work).await?;
    println!("{work}: {} names", names.len());
    println!("  {}", names.join("  "));
    Ok(())
}

async fn show(
    knowledge: &Knowledge,
    work: &str,
    limit: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    report(knowledge, work).await?;
    println!();
    println!("Unknown words this work leans on hardest:");
    for t in work_scripts::top_unknown(knowledge, work, limit).await? {
        let reading = if t.reading.is_empty() {
            String::new()
        } else {
            format!("【{}】", t.reading)
        };
        // `elsewhere` is encounters in everything read so far: a word already
        // met somewhere is a different proposition from one never seen.
        println!(
            "  {:>5}x  {}{}  {}  (met elsewhere: {})",
            t.count, t.headword, reading, t.status, t.elsewhere
        );
    }
    Ok(())
}

async fn report(knowledge: &Knowledge, work: &str) -> Result<(), Box<dyn std::error::Error>> {
    let c = work_scripts::coverage(knowledge, work).await?;
    if c.types == 0 {
        println!("no profile for {work} — run `jp-script profile` first");
        return Ok(());
    }
    let by_type = c.known_types as f64 / c.types as f64 * 100.0;
    println!();
    println!("{work}");
    println!(
        "  by token  {:.1}%  ({} of {} occurrences known)",
        c.token_coverage() * 100.0,
        c.known_tokens,
        c.tokens
    );
    println!(
        "  by type   {by_type:.1}%  ({} of {} distinct words known)",
        c.known_types, c.types
    );
    println!(
        "  unknown   {} judged unknown, {} never judged",
        c.unknown_types, c.new_types
    );
    println!("  (whole script, every route: a lower bound on one playthrough)");
    Ok(())
}
