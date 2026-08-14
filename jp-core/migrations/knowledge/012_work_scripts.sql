-- What a work is made of, from its script, before any of it has been read.
--
-- `work_terms` is the same shape and a different claim: it counts terms this
-- reader has *met* in a work, filled line by line as reading happens. This
-- counts terms the work *contains*, filled in one pass from the extracted
-- script. Folding the two together would assert the whole script had been read
-- the moment it was imported, which is what the per-work mined list, the
-- in-work decay metric and every `work_terms` join to `vocabulary` would then
-- be answering with.
--
-- Keyed on (headword, reading) like the ledger, and produced by the same
-- pipeline reading goes through, so the join to `vocabulary` is exact. No raw
-- spelling column: unlike `anki_notes` and `lookups`, these keys are the
-- tokenizer's own output rather than a spelling from outside.
--
-- The script text itself is not stored. Counts answer every question a profile
-- is for, and the extracted text is a rebuildable file on disk.
CREATE TABLE IF NOT EXISTS work_scripts (
    work        TEXT PRIMARY KEY,
    -- Occurrences that passed the wordhood gate: the coverage denominator, so
    -- that it and the numerator are counted by one rule.
    total_terms INTEGER NOT NULL,
    parsed_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS work_script_terms (
    work     TEXT NOT NULL,
    headword TEXT NOT NULL,
    reading  TEXT NOT NULL,
    count    INTEGER NOT NULL DEFAULT 0,

    PRIMARY KEY (work, headword, reading)
);

-- Coverage reads one work commonest-first; a term's spread reads every work.
CREATE INDEX IF NOT EXISTS idx_work_script_terms_work
    ON work_script_terms(work, count DESC);
CREATE INDEX IF NOT EXISTS idx_work_script_terms_term
    ON work_script_terms(headword, reading);
