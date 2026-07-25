-- read-stats' own tables: the ones that never join the knowledge layer.
--
-- Everything about *what was read* — lines, works, manual_sessions, anki_notes,
-- word_days, lookups — lives in jp-core's knowledge.db instead, because other
-- tools ask questions of it. What stays here is this app's own state: how it is
-- configured.
--
-- This file once also created `pauses`. Capture now stops at the source
-- instead of being filtered on read, so the table is retired by
-- db::retire_pauses rather than created here.

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
