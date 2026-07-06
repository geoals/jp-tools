CREATE TABLE IF NOT EXISTS dictionary_frequency (
    id INTEGER PRIMARY KEY,
    dictionary_id INTEGER NOT NULL REFERENCES dictionaries(id),
    term TEXT NOT NULL,
    frequency INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_dictionary_frequency_lookup ON dictionary_frequency(dictionary_id, term);
