-- kotodex-server-internal: the VNDB id each work's cover was fetched from, kept so
-- a deleted or missing cover file can be re-downloaded on startup.
--
-- Deliberately NOT a column on `works`: per spec/knowledge-db.md that table is
-- moving to the shared knowledge.db, while the VNDB cover source is a purely
-- kotodex-server display concern that stays local. `works` briefly carried a
-- vndb_id column once; it was dropped for the same reason (see db.rs).
CREATE TABLE IF NOT EXISTS work_covers (
    work_id INTEGER PRIMARY KEY,
    vndb_id TEXT NOT NULL
);
