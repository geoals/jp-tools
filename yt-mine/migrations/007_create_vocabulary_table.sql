CREATE TABLE IF NOT EXISTS vocabulary (
    id              INTEGER PRIMARY KEY,
    user_id         INTEGER NOT NULL DEFAULT 1,
    lemma           TEXT NOT NULL,
    reading         TEXT NOT NULL,
    pos             TEXT,
    status          TEXT NOT NULL DEFAULT 'seen',
    encounter_count INTEGER NOT NULL DEFAULT 0,
    first_seen      TEXT,
    last_seen       TEXT,
    source          TEXT,
    UNIQUE(user_id, lemma, reading)
);

CREATE INDEX IF NOT EXISTS idx_vocab_status ON vocabulary(user_id, status);
CREATE INDEX IF NOT EXISTS idx_vocab_lemma  ON vocabulary(user_id, lemma);
