-- Keep in sync with the schema bootstrap in vn-mine/vn-ws-logger.py: either
-- process may create the DB first, so both use CREATE ... IF NOT EXISTS.

-- Raw hooked lines from texthookers (source 'vn'). Reading time is *derived*
-- from inter-line gaps at query time, so thresholds stay tunable after the fact.
CREATE TABLE IF NOT EXISTS lines (
    id     INTEGER PRIMARY KEY,
    ts     REAL    NOT NULL,          -- epoch seconds
    chars  INTEGER NOT NULL,          -- counted codepoints, punctuation excluded (see charcount.rs; recomputable from text)
    text   TEXT,
    source TEXT    NOT NULL DEFAULT 'vn',
    work   TEXT,                     -- stamped from the current_work setting at capture
    -- 1 = retroactively cleared from the reader ("that wasn't reading"): lines
    -- hooked while finding a route, or a stretch re-read after skipping back.
    -- Soft rather than deleted, so the raw stream stays intact and it undoes.
    discarded INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_lines_ts ON lines(ts);

-- Explicitly logged sessions: physical books, imports, anything without a
-- line stream. chars may be estimated (pages × chars_per_page).
CREATE TABLE IF NOT EXISTS sessions (
    id       INTEGER PRIMARY KEY,
    start_ts REAL    NOT NULL,
    end_ts   REAL    NOT NULL,
    chars    INTEGER NOT NULL,
    source   TEXT    NOT NULL DEFAULT 'book',
    work     TEXT,
    pages    REAL,
    note     TEXT
);
CREATE INDEX IF NOT EXISTS idx_sessions_start_ts ON sessions(start_ts);

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Tracking-pause intervals (skipping scenes, replaying read text). Lines are
-- still captured raw; derivation drops those inside an interval, so a pause
-- can be corrected retroactively. end_ts NULL = currently paused.
CREATE TABLE IF NOT EXISTS pauses (
    id       INTEGER PRIMARY KEY,
    start_ts REAL NOT NULL,
    end_ts   REAL
);
