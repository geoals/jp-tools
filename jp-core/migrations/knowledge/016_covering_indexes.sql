-- Covering indexes for the four queries the highlighter builds itself from.
--
-- Each already searched an index and then fetched every matching row for one
-- more column — half a million of them for the wordhood gate. Carrying that
-- column in the index makes the whole pass index-only.
--
-- The existing (dictionary_id, term) indexes stay: they are prefixes of these,
-- but a smaller btree is still the better answer for a point lookup.
CREATE INDEX IF NOT EXISTS idx_dictionary_frequency_cover
    ON dictionary_frequency(dictionary_id, term, reading, frequency);

CREATE INDEX IF NOT EXISTS idx_dictionary_entries_cover
    ON dictionary_entries(dictionary_id, term, reading, score);

-- The kanji grid sums a lemma across every day it was read. The primary key
-- (lemma, date) orders the group correctly but holds no count, so the sum cost
-- one row fetch per day per lemma.
CREATE INDEX IF NOT EXISTS idx_word_days_lemma_count
    ON word_days(lemma, count);
