-- Yomitan lookup events, captured by proxying AnkiConnect (see ankiproxy.rs).
-- Yomitan's duplicate check queries Anki every time a popup is displayed, so a
-- proxied request carrying a term is one word looked up. Enabled by pointing
-- Yomitan's "Server address" at read-stats instead of AnkiConnect directly.
--
-- term is nullable: a request whose shape we don't recognize is never recorded,
-- but a schema-level NOT NULL would turn a future Yomitan change into a 500.
CREATE TABLE IF NOT EXISTS lookups (
    id   INTEGER PRIMARY KEY,
    ts   REAL NOT NULL,          -- epoch seconds
    term TEXT,                   -- dictionary form, as Yomitan sends it to Anki
    work TEXT                    -- stamped from current_work, like lines.work
);
CREATE INDEX IF NOT EXISTS idx_lookups_ts ON lookups(ts);
