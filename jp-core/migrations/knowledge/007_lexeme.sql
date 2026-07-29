-- The lexeme layer: which orthographic forms are the same *word*.
--
-- `vocabulary` keys on `(headword, reading)`, which is an orthographic form,
-- not a word. 零れ落ちる and こぼれ落ちる are two rows and one word; counting
-- rows therefore overstates known vocabulary. `dictionary_entries.sequence`
-- (JMdict `ent_seq`, carried by Jitendex) is what joins the two back together.
--
-- The column is added by an ALTER guarded in `jp_core::knowledge::migrate`,
-- since ALTER TABLE is not idempotent. Only the index lives here.
--
-- `seq_checked` marks a cached dictionary whose zip has already been re-read
-- for sequences, so a dictionary that simply has none (Sankoku) is not
-- re-parsed on every startup.

-- Reverse lookup: given a sequence, every form that carries it. This is the
-- direction the lexeme collapse reads in, and without it the join scans all
-- 518k entries per ledger row — the shape that once held the write lock for
-- six minutes (see the workspace CLAUDE.md).
CREATE INDEX IF NOT EXISTS idx_dictionary_entries_sequence
    ON dictionary_entries(dictionary_id, sequence);
