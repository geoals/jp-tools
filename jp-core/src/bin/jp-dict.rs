//! Dictionary management for the shared `knowledge.db`.
//!
//! Importing a Yomitan zip is cache warming, not a service's job: the result is
//! shared state that yt-mine, manga-mine and kotodex-server all read. Owned by any
//! one of them it becomes an ordering dependency between tools that are
//! otherwise independent — which is how adding a dictionary for the VN overlay
//! came to require booting yt-mine.
//!
//! So the services only open what is already cached (`Dictionary::load_cached`)
//! and this is the only thing that parses a zip.
//!
//!     jp-dict sync                 import every new zip in the dictionaries dir
//!     jp-dict import <zip>...      import named zips
//!     jp-dict list                 what is cached, and what each is for
//!     jp-dict reimport <id>        re-parse a cached zip after a parser fix
//!     jp-dict remove <id>          forget a cached dictionary and its entries
//!     jp-dict priority <id> <n>    who answers first
//!     jp-dict set-role <id> <role> master | standard | name | frequency |
//!                                  pitch | reference

use std::path::{Path, PathBuf};

use jp_core::dictionary::Dictionary;
use jp_core::knowledge::Knowledge;
use jp_core::knowledge::dictionaries::{self as db, DEFAULT_MASTER, Role};

const USAGE: &str = "\
jp-dict — manage the dictionaries in knowledge.db

usage:
  jp-dict sync                    import every zip in the dictionaries directory
  jp-dict import <zip>...         import the named zips
  jp-dict list                    list cached dictionaries and their roles
  jp-dict reimport <id>           re-parse a cached zip in place, keeping the
                                  id and the role (use after a parser fix)
  jp-dict remove <id>             forget a cached dictionary and its entries
                                  (the zip on disk is left alone)
  jp-dict import <zip>...         --role <role> overrides the guess
  jp-dict priority <id> <n>       who answers first: the popup's page order,
                                  and which frequency list is the reader's
  jp-dict set-role <id> <role>    role is master, standard, name, frequency,
                                  pitch or reference
                                  (standard: decides segmentation beside the
                                  master, never spelling or the word count)

options:
  --dir <path>   dictionaries directory (default: $KOTODEX_DICTIONARY_DIR,
                 else <repo>/dictionaries)
  --db <path>    knowledge.db (default: $KOTODEX_KNOWLEDGE_DB_PATH,
                 else ~/.local/share/kotodex/knowledge.db)

environment:
  KOTODEX_MASTER_DICTIONARY   title to promote to master when none is set yet,
                              matched as a substring. The master is the
                              vocabulary scale — which dictionary deserves to be
                              it is a judgement, so this only ever picks the
                              first one; `set-role` is how it is changed after.
";

fn main() -> std::process::ExitCode {
    // A single-threaded runtime: this is a batch job that awaits one SQLite
    // write at a time, and the default multi-thread pool buys it nothing.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build the async runtime");

    match rt.block_on(run()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("jp-dict: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let dir = take_option(&mut args, "--dir");
    let db_path = take_option(&mut args, "--db");
    let forced_role = match take_option(&mut args, "--role") {
        Some(r) => Some(parse_role(&r)?),
        None => None,
    };

    let (command, rest) = args
        .split_first()
        .ok_or_else(|| format!("no command given\n\n{USAGE}"))?;

    let knowledge = Knowledge::open(&resolve_db_path(db_path))
        .await
        .map_err(|e| format!("cannot open knowledge.db: {e}"))?;
    let pool = knowledge.pool();

    // Every command below but `list` and `help` changes what the highlighter is
    // built from, so the derived cache is rewritten once after whichever ran.
    let derives = !matches!(command.as_str(), "list" | "help" | "--help" | "-h");

    let outcome = match command.as_str() {
        "sync" => {
            let dir = resolve_dictionary_dir(dir);
            let zips = zips_in(&dir)?;
            if zips.is_empty() {
                println!("no .zip dictionaries in {}", dir.display());
            } else {
                import_all(pool, &zips, forced_role).await?;
                ensure_master(pool).await?;
            }
            list(pool).await
        }
        "import" => {
            if rest.is_empty() {
                return Err(format!("import needs at least one zip\n\n{USAGE}"));
            }
            let zips: Vec<PathBuf> = rest.iter().map(PathBuf::from).collect();
            for zip in &zips {
                if !zip.exists() {
                    return Err(format!("no such file: {}", zip.display()));
                }
            }
            import_all(pool, &zips, forced_role).await?;
            ensure_master(pool).await?;
            list(pool).await
        }
        "list" => list(pool).await,
        "priority" => {
            let [id, priority] = rest else {
                return Err(format!("priority needs an id and a number\n\n{USAGE}"));
            };
            let id: i64 = id.parse().map_err(|_| format!("not an id: {id}"))?;
            let priority: i64 = priority
                .parse()
                .map_err(|_| format!("not a number: {priority}"))?;
            db::set_priority(pool, id, priority)
                .await
                .map_err(|e| format!("cannot set the priority: {e}"))?;
            list(pool).await
        }
        "reimport" => {
            let [id] = rest else {
                return Err(format!("reimport needs an id\n\n{USAGE}"));
            };
            let id: i64 = id.parse().map_err(|_| format!("not an id: {id}"))?;
            let cached = db::list_dictionaries(pool)
                .await
                .map_err(|e| format!("cannot read the dictionary list: {e}"))?;
            let target = cached
                .iter()
                .find(|d| d.id == id)
                .ok_or_else(|| format!("no cached dictionary with id {id}"))?;
            let path = PathBuf::from(&target.source_path);
            if !path.exists() {
                return Err(format!("the zip is gone: {}", target.source_path));
            }
            let count = Dictionary::reimport(pool, id, &path)
                .await
                .map_err(|e| format!("cannot re-import {}: {e}", target.title))?;
            println!("{}  {count} entries", target.title);
            Ok(())
        }
        "remove" => {
            let [id] = rest else {
                return Err(format!("remove needs an id\n\n{USAGE}"));
            };
            let id: i64 = id.parse().map_err(|_| format!("not an id: {id}"))?;
            let cached = db::list_dictionaries(pool)
                .await
                .map_err(|e| format!("cannot read the dictionary list: {e}"))?;
            let target = cached
                .iter()
                .find(|d| d.id == id)
                .ok_or_else(|| format!("no cached dictionary with id {id}"))?;
            if target.role == Role::Master {
                return Err(format!(
                    "{} is the master dictionary — set another one master first",
                    target.title
                ));
            }
            let rows = db::remove_dictionary(pool, id)
                .await
                .map_err(|e| format!("cannot remove {}: {e}", target.title))?;
            println!("{}  {rows} entries forgotten", target.title);
            println!();
            println!("`jp-dict sync` re-imports it if the zip is still in the");
            println!("dictionaries directory. Re-derive the ledger afterwards:");
            println!("which dictionaries hold a term is cached on its row.");
            list(pool).await
        }
        "set-role" => {
            let [id, role] = rest else {
                return Err(format!("set-role needs an id and a role\n\n{USAGE}"));
            };
            let id: i64 = id.parse().map_err(|_| format!("not an id: {id}"))?;
            let parsed = parse_role(role)?;
            let cached = db::list_dictionaries(pool)
                .await
                .map_err(|e| format!("cannot read the dictionary list: {e}"))?;
            let target = cached
                .iter()
                .find(|d| d.id == id)
                .ok_or_else(|| format!("no cached dictionary with id {id}"))?;
            // Exactly one master, so promoting one demotes the incumbent.
            if parsed == Role::Master {
                for other in cached
                    .iter()
                    .filter(|d| d.role == Role::Master && d.id != id)
                {
                    db::set_role(pool, other.id, Role::Reference)
                        .await
                        .map_err(|e| format!("cannot demote {}: {e}", other.title))?;
                    println!("{} is now reference", other.title);
                }
            }
            db::set_role(pool, id, parsed)
                .await
                .map_err(|e| format!("cannot set the role: {e}"))?;
            println!("{} is now {}", target.title, parsed.as_str());
            Ok(())
        }
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command: {other}\n\n{USAGE}")),
    };

    outcome?;
    if derives {
        refresh_derived_cache(pool).await;
    }
    Ok(())
}

/// Rewrite the collections the highlighter is built from, if the dictionaries
/// moved.
///
/// **This is their only writer.** They are derived from the dictionaries and
/// nothing else, and every command above changes one — so this is the one place
/// that knows. A service reads the cache and never fills it; skipping this costs
/// the reader the seconds of deriving them itself, and nothing more, which is
/// why a failure here warns rather than failing the import that just succeeded.
async fn refresh_derived_cache(pool: &sqlx::SqlitePool) {
    match jp_core::highlight::derived::rebuild(pool).await {
        Ok(true) => println!("derived cache rebuilt"),
        Ok(false) => {}
        Err(e) => eprintln!("warning: cannot write the derived cache: {e}"),
    }
}

/// What a freshly imported zip is for, from what it turned out to contain.
///
/// A frequency list and a pitch dictionary hold no term entries at all, which
/// is the same signal the popup already uses to keep them off the page. A zip
/// with definitions gets `reference` and stays there until someone decides
/// otherwise — `ensure_master` is what promotes one to master.
async fn guess_role(pool: &sqlx::SqlitePool, id: i64) -> Result<Role, sqlx::Error> {
    let count = async |table: &str| -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(&format!(
            "SELECT EXISTS(SELECT 1 FROM {table} WHERE dictionary_id = ?)"
        ))
        .bind(id)
        .fetch_one(pool)
        .await
    };
    if count("dictionary_entries").await? > 0 {
        return Ok(Role::Reference);
    }
    if count("dictionary_frequency").await? > 0 {
        return Ok(Role::Frequency);
    }
    if count("dictionary_pitch").await? > 0 {
        return Ok(Role::Pitch);
    }
    Ok(Role::Reference)
}

/// Import every zip that is not cached yet, and repoint any whose file has
/// moved. Both are keyed on `source_path`, which is the cache key.
async fn import_all(
    pool: &sqlx::SqlitePool,
    zips: &[PathBuf],
    forced_role: Option<Role>,
) -> Result<(), String> {
    let cached = db::list_dictionaries(pool)
        .await
        .map_err(|e| format!("cannot read the dictionary list: {e}"))?;

    for zip in zips {
        let path = zip
            .canonicalize()
            .map_err(|e| format!("cannot resolve {}: {e}", zip.display()))?;
        let path_str = path.to_string_lossy().to_string();

        // A zip cached under a different path — historically a bare relative
        // filename, resolved against whichever service's working directory
        // imported it. Repoint rather than re-import: same dictionary, and
        // re-importing would cost a 400k-row pass and leave a duplicate row.
        if !cached.iter().any(|d| d.source_path == path_str) {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string());
            if let Some(moved) = name.and_then(|n| {
                cached.iter().find(|d| {
                    Path::new(&d.source_path)
                        .file_name()
                        .is_some_and(|f| f == n.as_str())
                })
            }) {
                db::set_source_path(pool, moved.id, &path_str)
                    .await
                    .map_err(|e| format!("cannot repoint {}: {e}", moved.title))?;
                println!("{}  repointed to {}", moved.title, path.display());
                continue;
            }
        }

        let fresh = !cached.iter().any(|d| d.source_path == path_str);
        match Dictionary::load_or_import(pool, &path).await {
            // Both the cached and freshly-imported cases log through `tracing`,
            // which is not initialised here; say what happened either way.
            Ok(dict) => {
                // A guess only on a new row: a role is a decision, and
                // re-asserting one over `set-role` on every sync would
                // silently undo it. `--role` is that decision, so it always
                // applies.
                let id = db::list_dictionaries(pool)
                    .await
                    .map_err(|e| format!("cannot read the dictionary list: {e}"))?
                    .into_iter()
                    .find(|d| d.source_path == path_str)
                    .map(|d| d.id);
                let role = match (forced_role, fresh, id) {
                    (Some(r), _, Some(_)) => Some(r),
                    (None, true, Some(id)) => Some(
                        guess_role(pool, id)
                            .await
                            .map_err(|e| format!("cannot inspect {}: {e}", dict.title()))?,
                    ),
                    _ => None,
                };
                // A new row lands at priority 0, which would put it ahead of
                // everything the backfill numbered by id. Install order is the
                // default, so a new dictionary goes last.
                if fresh && let Some(id) = id {
                    db::set_priority(pool, id, id)
                        .await
                        .map_err(|e| format!("cannot order {}: {e}", dict.title()))?;
                }
                match (role, id) {
                    (Some(role), Some(id)) => {
                        db::set_role(pool, id, role)
                            .await
                            .map_err(|e| format!("cannot set the role of {}: {e}", dict.title()))?;
                        println!("{}  ok ({})", dict.title(), role.as_str());
                    }
                    _ => println!("{}  ok", dict.title()),
                }
            }
            Err(e) => return Err(format!("cannot import {}: {e}", path.display())),
        }
    }
    Ok(())
}

/// Mark the master unless one is already set. Not forced on every run: the
/// role is a decision, and `set-role` is how it gets changed — re-asserting the
/// default here would silently undo it.
async fn ensure_master(pool: &sqlx::SqlitePool) -> Result<(), String> {
    let existing = db::master(pool)
        .await
        .map_err(|e| format!("cannot read the master dictionary: {e}"))?;
    if let Some(master) = existing {
        println!("master: {}", master.title);
        return Ok(());
    }
    let marker =
        std::env::var("KOTODEX_MASTER_DICTIONARY").unwrap_or_else(|_| DEFAULT_MASTER.to_string());
    match db::ensure_master(pool, &marker).await {
        Ok(Some(id)) => {
            let title = db::master(pool)
                .await
                .ok()
                .flatten()
                .map(|d| d.title)
                .unwrap_or_else(|| id.to_string());
            if title == marker || marker.is_empty() {
                println!("master: {title}");
            } else {
                println!("master: {title} ({marker} is not cached)");
            }
        }
        Ok(None) => eprintln!(
            "warning: no master dictionary — nothing is cached to fall back to. \
             The vocabulary count will read zero until one is set."
        ),
        Err(e) => return Err(format!("cannot set the master dictionary: {e}")),
    }
    Ok(())
}

async fn list(pool: &sqlx::SqlitePool) -> Result<(), String> {
    let cached = db::list_dictionaries(pool)
        .await
        .map_err(|e| format!("cannot read the dictionary list: {e}"))?;
    if cached.is_empty() {
        println!("no dictionaries cached — run: jp-dict sync");
        return Ok(());
    }
    // Role before title: a CJK title is double-width, so padding it into a
    // column by character count misaligns everything after it.
    for d in &cached {
        let missing = if Path::new(&d.source_path).exists() {
            ""
        } else {
            "  (zip missing)"
        };
        println!(
            "{:>3}  {:>4}  {:<9}  {}{}",
            d.id,
            d.priority,
            d.role.as_str(),
            d.title,
            missing
        );
    }
    Ok(())
}

fn zips_in(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| {
        format!(
            "cannot read the dictionaries directory {}: {e}",
            dir.display()
        )
    })?;
    let mut zips: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")))
        .collect();
    zips.sort();
    Ok(zips)
}

/// `--dir`, else `KOTODEX_DICTIONARY_DIR`, else `dictionaries/` beside the
/// workspace this binary was built from.
fn resolve_dictionary_dir(flag: Option<String>) -> PathBuf {
    if let Some(dir) = flag {
        return PathBuf::from(dir);
    }
    if let Ok(dir) = std::env::var("KOTODEX_DICTIONARY_DIR") {
        return PathBuf::from(dir);
    }
    jp_core::install::install_root().join("dictionaries")
}

fn resolve_db_path(flag: Option<String>) -> String {
    flag.or_else(|| std::env::var("KOTODEX_KNOWLEDGE_DB_PATH").ok())
        .unwrap_or_else(|| {
            jp_core::install::data_dir()
                .join("knowledge.db")
                .display()
                .to_string()
        })
}

/// `Role::parse` treats anything unknown as `reference`, which would silently
/// demote a dictionary on a typo.
fn parse_role(role: &str) -> Result<Role, String> {
    match role {
        "master" => Ok(Role::Master),
        "standard" => Ok(Role::Standard),
        "name" => Ok(Role::Name),
        "frequency" => Ok(Role::Frequency),
        "pitch" => Ok(Role::Pitch),
        "reference" => Ok(Role::Reference),
        other => Err(format!(
            "unknown role: {other} (master, standard, name, frequency, pitch, reference)"
        )),
    }
}

fn take_option(args: &mut Vec<String>, name: &str) -> Option<String> {
    let at = args.iter().position(|a| a == name)?;
    if at + 1 >= args.len() {
        return None;
    }
    let value = args.remove(at + 1);
    args.remove(at);
    Some(value)
}
