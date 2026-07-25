//! `work_covers` — where a work's cover image came from.
//!
//! Deliberately a separate table rather than a column on `works`: the VNDB id
//! is read-stats' own bookkeeping (it exists so a cover file lost from disk can
//! be re-fetched at startup), while `works` is shared knowledge-layer data. The
//! cover *filename* does live on `works`, since that is what the API serves.

use sqlx::{Row, SqlitePool};

/// Remember which VNDB id a work's cover came from, so a lost cover file can be
/// re-fetched. Lives in read-stats' own `work_covers`, not on `works`.
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
pub async fn fetch_work_covers(
    pool: &SqlitePool,
) -> Result<Vec<(i64, String, Option<String>)>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT wc.work_id, wc.vndb_id, w.cover_path
         FROM work_covers wc JOIN works w ON w.id = wc.work_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get("work_id"), r.get("vndb_id"), r.get("cover_path")))
        .collect())
}
