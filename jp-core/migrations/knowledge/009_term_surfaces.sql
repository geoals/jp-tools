-- How each term was actually written on the page.
--
-- The ledger keys on Sudachi's *normalized* form, which is what merges
-- できる/出来る and うかがう/窺う into one row. That is the right key to count
-- on, but it throws away the orthography, and the orthography is what a triage
-- verdict is about: 窺う and うかがう are the same word and not the same
-- question. 何 written ナニ is a third.
--
-- So the surface is kept beside the count, plus one line id per surface. The
-- line is the evidence: a row can be checked against the sentence it came from,
-- which is the only way to tell a real word from a tokenizer artefact without
-- leaving the queue.
--
-- Additive behind its own watermark, like `work_terms`. Session text has no
-- line id, so `line_id` is nullable — the count still lands.
CREATE TABLE IF NOT EXISTS term_surfaces (
    headword TEXT NOT NULL,
    reading  TEXT NOT NULL,
    -- As the text spelt it, inflection and all: 出来る, できれ, ナニ.
    surface  TEXT NOT NULL,
    count    INTEGER NOT NULL DEFAULT 0,
    -- The first line this surface was met in. First rather than latest so a
    -- rebuild lands on the same line every time.
    line_id  INTEGER,

    PRIMARY KEY (headword, reading, surface)
);

-- The only read: every surface of one term, commonest first.
CREATE INDEX IF NOT EXISTS idx_term_surfaces_term
    ON term_surfaces(headword, reading, count DESC);
