-- The reading side of knowledge.db: what has been read, and what was looked up
-- while reading it.
--
-- These tables were read-stats' until spec/knowledge-db.md placed them here.
-- The reason is not that read-stats stopped needing them but that they are the
-- raw material every other tool's questions are asked against: a word's
-- encounter count is derived from `lines`, a kotodex encounter map aggregates by
-- `works`, and "have I mined this" is `anki_notes`. read-stats writes into this
-- database; it is not a pure reader, and that is by design.
--
-- Keep in sync with the schema bootstrap in vn-mine/vn-ws-logger.py — either
-- process may create the DB first, so both use CREATE ... IF NOT EXISTS.

-- The source dimension: one row per VN, book or article. Joined to
-- lines.work / manual_sessions.work by exact title string, because the
-- texthooker stamps a title and has no way to look up a foreign key.
--
-- total_chars is pasted manually from jpdb (no public API); VNDB is used only
-- as a one-shot cover fetch at save time, nothing from it is stored.
CREATE TABLE IF NOT EXISTS works (
    id          INTEGER PRIMARY KEY,
    title       TEXT NOT NULL UNIQUE,
    total_chars INTEGER,
    cover_path  TEXT,                  -- filename under <db dir>/covers/
    status      TEXT NOT NULL DEFAULT 'reading',  -- reading | queued | finished | dropped
    queue_pos   INTEGER,
    vn_window   TEXT                   -- substring of this VN's window title
);

-- Raw hooked lines from texthookers (source 'vn'). Reading time is *derived*
-- from inter-line gaps at query time, so thresholds stay tunable after the fact.
CREATE TABLE IF NOT EXISTS lines (
    id     INTEGER PRIMARY KEY,
    ts     REAL    NOT NULL,          -- epoch seconds
    chars  INTEGER NOT NULL,          -- counted codepoints, punctuation excluded (jp_core::text::chars; recomputable from text)
    text   TEXT,
    source TEXT    NOT NULL DEFAULT 'vn',
    work   TEXT,                      -- stamped from the current_work setting at capture
    -- 1 = retroactively cleared from the reader ("that wasn't reading"): lines
    -- hooked while finding a route, or a stretch re-read after skipping back.
    -- Soft rather than deleted, so the raw stream stays intact and it undoes.
    discarded INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_lines_ts ON lines(ts);

-- Explicitly logged sessions: physical books, imports, anything without a
-- line stream. chars may be estimated (pages × chars_per_page).
--
-- Named `manual_sessions`, not `sessions`: a *derived* session (a sitting cut
-- out of the line stream) is the other meaning of that word in this codebase,
-- and it is computed, never stored.
--
-- `content` holds the actual text read, when there is one to paste (an online
-- article, an ebook chapter). It makes `chars` exact rather than estimated —
-- counted by the same `jp_core::text::count_chars` the line stream uses — and
-- it is what a later import tokenizes into the knowledge layer's counts. It
-- lives on this row and is deliberately *not* expanded into `lines`: these are
-- not hooked lines and have no per-line timestamps to invent.
CREATE TABLE IF NOT EXISTS manual_sessions (
    id       INTEGER PRIMARY KEY,
    start_ts REAL    NOT NULL,
    end_ts   REAL    NOT NULL,
    chars    INTEGER NOT NULL,
    source   TEXT    NOT NULL DEFAULT 'book',
    work     TEXT,
    pages    REAL,
    note     TEXT,
    content  TEXT,
    url      TEXT
);
CREATE INDEX IF NOT EXISTS idx_manual_sessions_start_ts ON manual_sessions(start_ts);

-- Snapshot of the mined Anki deck (read-only mirror, replaced wholesale on
-- each refresh). The note id doubles as creation time in epoch milliseconds.
-- Anki owns this fact; the mirror exists so a lookup can be joined against it
-- without a round trip to a program that may not be running.
CREATE TABLE IF NOT EXISTS anki_notes (
    note_id INTEGER PRIMARY KEY,
    vocab   TEXT NOT NULL           -- VocabKanji field: dictionary form
);

-- Per-day content-word counts tokenized incrementally from the raw line
-- stream. Deck-independent, so words mined later still match past reading.
CREATE TABLE IF NOT EXISTS word_days (
    lemma TEXT NOT NULL,            -- Sudachi dictionary form
    date  TEXT NOT NULL,            -- rollover-adjusted ISO day
    count INTEGER NOT NULL,
    PRIMARY KEY (lemma, date)
);

-- Yomitan lookup events, captured by proxying AnkiConnect (see ankiproxy.rs).
-- Yomitan's duplicate check queries Anki every time a popup is displayed, so a
-- proxied request carrying a term is one word looked up.
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

-- Lookup outcomes join lookups.term against anki_notes.vocab per distinct term.
-- Both sides grow without bound (one row per lookup, one per mined card), so
-- the join needs indexes to stay cheap as the history builds up.
CREATE INDEX IF NOT EXISTS idx_anki_notes_vocab ON anki_notes(vocab);
CREATE INDEX IF NOT EXISTS idx_lookups_term ON lookups(term);
