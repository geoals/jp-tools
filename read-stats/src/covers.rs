//! Cover cache reconciliation.
//!
//! A work's cover file lives on disk under `covers_dir`; the VNDB id it was
//! fetched from is remembered in `work_covers` (see `db`). Keeping the id lets
//! us re-download a cover whose file has gone missing — on startup, or after a
//! failed/interrupted fetch — without the user re-entering the id.

use std::path::Path;

use sqlx::SqlitePool;
use tracing::{info, warn};

use crate::{db, error::AppError, vndb};

/// Fetch `vndb_id`'s cover into `covers_dir` as `w{work_id}.<ext>`, record the
/// filename on the work row and the id in `work_covers`. Returns the filename.
pub async fn fetch_and_store(
    http: &reqwest::Client,
    pool: &SqlitePool,
    covers_dir: &Path,
    work_id: i64,
    vndb_id: &str,
) -> Result<String, AppError> {
    let url = vndb::fetch_cover_url(http, vndb_id).await?;
    let filename = vndb::download_cover(http, &url, covers_dir, &format!("w{work_id}")).await?;
    db::set_work_cover(pool, work_id, Some(&filename)).await?;
    db::set_work_cover_vndb(pool, work_id, vndb_id).await?;
    Ok(filename)
}

/// Re-download any cover whose file is gone but whose VNDB id we still have.
/// Cheap: one existence check per remembered work, network only for the ones
/// actually missing. Meant to run once at startup and best-effort — a VNDB
/// hiccup just leaves that cover missing until the next boot.
pub async fn reconcile_missing(http: &reqwest::Client, pool: &SqlitePool, covers_dir: &Path) {
    let rows = match db::fetch_work_covers(pool).await {
        Ok(rows) => rows,
        Err(e) => {
            warn!(error = %e, "cover reconcile: could not read work_covers");
            return;
        }
    };
    for (work_id, vndb_id, cover_path) in rows {
        let present = cover_path
            .as_ref()
            .is_some_and(|f| covers_dir.join(f).exists());
        if present {
            continue;
        }
        match fetch_and_store(http, pool, covers_dir, work_id, &vndb_id).await {
            Ok(f) => info!(work_id, vndb_id, file = %f, "cover reconcile: re-fetched missing cover"),
            Err(e) => warn!(work_id, vndb_id, error = %e, "cover reconcile: re-fetch failed"),
        }
    }
}
