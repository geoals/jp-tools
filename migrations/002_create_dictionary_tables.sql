CREATE TABLE IF NOT EXISTS dictionaries (
    id INTEGER PRIMARY KEY,
    title TEXT NOT NULL,
    source_path TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS dictionary_entries (
    id INTEGER PRIMARY KEY,
    dictionary_id INTEGER NOT NULL REFERENCES dictionaries(id),
    term TEXT NOT NULL,
    reading TEXT NOT NULL DEFAULT '',
    score INTEGER NOT NULL DEFAULT 0,
    definitions_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_dictionary_entries_term ON dictionary_entries(term);
CREATE INDEX IF NOT EXISTS idx_dictionary_entries_dict ON dictionary_entries(dictionary_id);
