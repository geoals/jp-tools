//! `work_covers` — where a work's cover image came from.
//!
//! Deliberately a separate table rather than a column on `works`: the VNDB id
//! is kotodex-server's own bookkeeping (it exists so a cover file lost from disk can
//! be re-fetched at startup), while `works` is shared knowledge-layer data. The
//! cover *filename* does live on `works`, since that is what the API serves.

use std::collections::HashMap;

use jp_core::knowledge::Knowledge;
use sqlx::{Row, SqlitePool};

/// Remember which VNDB id a work's cover came from, so a lost cover file can be
/// re-fetched. Lives in kotodex-server's own `work_covers`, not on `works`.
pub async fn set_work_cover_vndb(
    pool: &SqlitePool,
    work_id: i64,
    vndb_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO work_covers (work_id, vndb_id) VALUES (?, ?)
         ON CONFLICT(work_id) DO UPDATE SET vndb_id = excluded.vndb_id",
    )
    .bind(work_id)
    .bind(vndb_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Forget a work's cover source (cover removed, or work deleted).
pub async fn clear_work_cover_vndb(pool: &SqlitePool, work_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM work_covers WHERE work_id = ?")
        .bind(work_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Every work that has a remembered cover source, with its current cover
/// filename — the input to the startup "re-fetch anything missing" pass.
///
/// Joined in memory rather than in SQL: `work_covers` is local and `works` is
/// in the shared knowledge database. There is one row per work with art, so the
/// cost of doing it this way is nothing and the cost of an ATTACH would be a
/// second connection-setup path.
pub async fn fetch_work_covers(
    pool: &SqlitePool,
    knowledge: &Knowledge,
) -> Result<Vec<(i64, String, Option<String>)>, sqlx::Error> {
    let sources = sqlx::query("SELECT work_id, vndb_id FROM work_covers")
        .fetch_all(pool)
        .await?;
    let covers: HashMap<i64, Option<String>> = super::works::fetch_works_meta(knowledge)
        .await?
        .into_iter()
        .map(|w| (w.id, w.cover_path))
        .collect();
    Ok(sources
        .iter()
        .filter_map(|r| {
            let work_id: i64 = r.get("work_id");
            // A cover source whose work has been deleted is dropped, which is
            // what the SQL join did too.
            covers
                .get(&work_id)
                .map(|cover| (work_id, r.get("vndb_id"), cover.clone()))
        })
        .collect())
}
