-- Which work each term was met in, and how often.
--
-- `word_days` answers "when did I meet this word" and `vocabulary` answers
-- "what do I know about it". Neither can answer "what is this VN made of", and
-- that question is what a per-work vocabulary view is: how many distinct words,
-- how many of them I have judged known, and which unknown ones this particular
-- work leans on hardest.
--
-- Keyed on (headword, reading) like the ledger rather than on lemma like
-- `word_days`, so the join to `vocabulary` is exact and a homograph does not
-- borrow its twin's status. `work` is the same exact title `lines.work` carries
-- — the join key everything else in the workspace already uses.
--
-- Additive, like `word_days`: counts are incremented behind a watermark of
-- their own (`vocab_works_through_line_id` / `_session_id` in read-stats'
-- settings), which is what makes a rebuild able to re-derive this sink without
-- double-counting the others.
CREATE TABLE IF NOT EXISTS work_terms (
    headword TEXT NOT NULL,
    reading  TEXT NOT NULL,
    work     TEXT NOT NULL,
    count    INTEGER NOT NULL DEFAULT 0,

    PRIMARY KEY (headword, reading, work)
);

-- The per-work views read every term of one work, commonest first; the
-- "met here and nowhere else" list reads every work of one term.
CREATE INDEX IF NOT EXISTS idx_work_terms_work ON work_terms(work, count DESC);
CREATE INDEX IF NOT EXISTS idx_work_terms_term ON work_terms(headword, reading);
