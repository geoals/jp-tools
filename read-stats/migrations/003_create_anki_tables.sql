-- Snapshot of the mined Anki deck (read-only mirror, replaced wholesale on
-- each refresh). The note id doubles as creation time in epoch milliseconds.
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
