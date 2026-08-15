//! The cast of a work, imported rather than inferred.
//!
//! The tokenizer's largest known error is the name filter: it can only ask
//! Sudachi's 固有名詞 tag, and Sudachi has no entry for most of a VN's cast.
//! Where it has none the name does not merely go untagged, it is *split* —
//! 世凪 arrives as 世 + 凪 and the ledger fills with a word the text never
//! used.
//!
//! A list per work fixes what no rule can, because the cast is knowable before
//! the work is read. See the migration for why it is scoped per work.

use std::collections::HashSet;

use sqlx::Row;

use super::Knowledge;

/// Replace one source's names for a work, leaving the other source's alone.
///
/// Names added by hand survive a refetch, and a refetch is the only way to
/// drop a name VNDB no longer lists.
pub async fn replace(
    k: &Knowledge,
    work: &str,
    source: &str,
    names: &[String],
) -> Result<u64, sqlx::Error> {
    let mut tx = k.pool().begin().await?;
    sqlx::query("DELETE FROM work_names WHERE work = ? AND source = ?")
        .bind(work)
        .bind(source)
        .execute(&mut *tx)
        .await?;
    let mut n = 0;
    for name in names {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        n += sqlx::query(
            "INSERT INTO work_names (work, name, source) VALUES (?, ?, ?) \
             ON CONFLICT(work, name) DO UPDATE SET source = excluded.source",
        )
        .bind(work)
        .bind(name)
        .bind(source)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    tx.commit().await?;
    Ok(n)
}

pub async fn of_work(k: &Knowledge, work: &str) -> Result<Vec<String>, sqlx::Error> {
    let rows = sqlx::query("SELECT name FROM work_names WHERE work = ? ORDER BY LENGTH(name) DESC")
        .bind(work)
        .fetch_all(k.pool())
        .await?;
    Ok(rows.iter().map(|r| r.get("name")).collect())
}

/// Every work's names at once, for the tokenizer.
///
/// One tokenizer serves a pass over lines from several works, so it cannot
/// hold a per-work set. The union is what it gets. That is safe in a way the
/// same union would not be if these were guessed: each row is a name someone
/// published for a real character, and the cost of treating one as a name in a
/// work whose text meant the ordinary word is one term, against 2,385
/// fabricated sightings from splitting it.
pub async fn all(k: &Knowledge) -> Result<Vec<String>, sqlx::Error> {
    let rows = sqlx::query("SELECT DISTINCT name FROM work_names")
        .fetch_all(k.pool())
        .await?;
    let mut names: HashSet<String> = HashSet::new();
    for row in &rows {
        let name: String = row.get("name");
        names.extend(parts(&name));
        names.insert(name);
    }
    Ok(names.into_iter().collect())
}

/// The halves of a full name, which is what the text mostly uses.
///
/// A cast list gives ウィリアム・シェイクスピア and the script says
/// シェイクスピア, オリヴィア・ベリー and the script says オリヴィア. Only for
/// the tokenizer: the stored list stays what was imported, so a refetch still
/// diffs against VNDB rather than against what was derived from it.
///
/// One character is never a part. ピーチ・ザ・ビッチ would otherwise teach it
/// that ザ is somebody.
fn parts(name: &str) -> impl Iterator<Item = String> {
    name.split(['・', '＝'])
        .filter(|p| p.chars().count() > 1)
        .map(str::to_string)
        .collect::<Vec<_>>()
        .into_iter()
}
