-- The dictionary-derived collections the highlighter is built from, cached
-- whole. See `jp_core::highlight::derived`, which owns the format.
--
-- `fingerprint` is what the dictionaries looked like when the payload was
-- written; a reader that computes a different one builds from the rows instead.
-- One row per section — see `SECTIONS` there — so that no write holds the write
-- lock for the whole 50 MB and each section decodes on a thread of its own. A
-- cache whose sections disagree about the fingerprint is no cache.
CREATE TABLE IF NOT EXISTS derived_cache (
    name        TEXT PRIMARY KEY,
    fingerprint TEXT NOT NULL,
    payload     BLOB NOT NULL,
    built_ts    REAL NOT NULL
);
