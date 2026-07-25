-- read-stats' own tables: the ones that never join the knowledge layer.
--
-- Everything about *what was read* — lines, works, manual_sessions, anki_notes,
-- word_days, lookups — lives in jp-core's knowledge.db instead, because other
-- tools ask questions of it. What stays here is this app's own state: how it is
-- configured, and which spans of time it was told not to count.

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Tracking-pause intervals (skipping scenes, replaying read text). Lines are
-- still captured raw; derivation drops those inside an interval, so a pause
-- can be corrected retroactively. end_ts NULL = currently paused.
--
-- Derivation input, not knowledge: which lines count is a read-stats policy,
-- and a tool asking "how often have I met this word" has no use for it.
CREATE TABLE IF NOT EXISTS pauses (
    id       INTEGER PRIMARY KEY,
    start_ts REAL NOT NULL,
    end_ts   REAL
);
