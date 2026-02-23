CREATE TABLE IF NOT EXISTS dictionary_pitch (
    id INTEGER PRIMARY KEY,
    dictionary_id INTEGER NOT NULL REFERENCES dictionaries(id),
    term TEXT NOT NULL,
    reading TEXT NOT NULL,
    positions_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_dictionary_pitch_term ON dictionary_pitch(term);
CREATE INDEX IF NOT EXISTS idx_dictionary_pitch_dict ON dictionary_pitch(dictionary_id);
