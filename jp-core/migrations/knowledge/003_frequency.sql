CREATE TABLE IF NOT EXISTS dictionary_frequency (
    id INTEGER PRIMARY KEY,
    dictionary_id INTEGER NOT NULL REFERENCES dictionaries(id),
    term TEXT NOT NULL,
    -- Empty when the source gave none, and for rows imported before this
    -- column existed. A corpus counts (term, reading) pairs separately: 私 is
    -- rank 47 as わたくし and 182 as わたし.
    reading TEXT NOT NULL DEFAULT '',
    frequency INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_dictionary_frequency_lookup ON dictionary_frequency(dictionary_id, term);
