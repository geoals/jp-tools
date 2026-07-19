-- Lookup outcomes join lookups.term against anki_notes.vocab per distinct term.
-- Both sides grow without bound (one row per lookup, one per mined card), so
-- the join needs indexes to stay cheap as the history builds up.
CREATE INDEX IF NOT EXISTS idx_anki_notes_vocab ON anki_notes(vocab);
CREATE INDEX IF NOT EXISTS idx_lookups_term ON lookups(term);
