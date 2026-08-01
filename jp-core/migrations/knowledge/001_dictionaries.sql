-- role decides which questions a dictionary may answer: `master` alone defines
-- the vocabulary denominator, `name` marks proper nouns, `reference` (the
-- default for any new import) counts only toward "is this a word at all".
-- See jp_core::knowledge::dictionaries for why they cannot be pooled.
CREATE TABLE IF NOT EXISTS dictionaries (
    id INTEGER PRIMARY KEY,
    title TEXT NOT NULL,
    source_path TEXT NOT NULL UNIQUE,
    role TEXT NOT NULL DEFAULT 'reference'
);

CREATE TABLE IF NOT EXISTS dictionary_entries (
    id INTEGER PRIMARY KEY,
    dictionary_id INTEGER NOT NULL REFERENCES dictionaries(id),
    term TEXT NOT NULL,
    reading TEXT NOT NULL DEFAULT '',
    score INTEGER NOT NULL DEFAULT 0,
    definitions_json TEXT NOT NULL,
    -- Yomitan's deinflection rules (v5, v1, adj-i, or empty): whether the
    -- dictionary calls this entry a conjugatable lemma. 許す has it, 許せ does
    -- not, and both are headwords.
    rules TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_dictionary_entries_lookup ON dictionary_entries(dictionary_id, term);
-- The same seek by reading, for the wordhood gate: a kana headword (いう,
-- できる) is a real word the dictionary happens to list in kanji, and matching
-- only on `term` filed all of them as tokenizer noise.
CREATE INDEX IF NOT EXISTS idx_dictionary_entries_reading ON dictionary_entries(dictionary_id, reading);
