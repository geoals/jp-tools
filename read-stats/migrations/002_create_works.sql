-- Work metadata, joined to lines.work / sessions.work by exact title string.
-- total_chars is pasted manually from jpdb (no public API); VNDB is used only
-- as a one-shot cover fetch at save time, nothing from it is stored.
CREATE TABLE IF NOT EXISTS works (
    id          INTEGER PRIMARY KEY,
    title       TEXT NOT NULL UNIQUE,
    total_chars INTEGER,
    cover_path  TEXT,                  -- filename under <db dir>/covers/
    status      TEXT NOT NULL DEFAULT 'reading',  -- reading | queued | finished | dropped
    queue_pos   INTEGER
);
