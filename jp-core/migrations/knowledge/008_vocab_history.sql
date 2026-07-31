-- The history of what I have said I know.
--
-- `vocabulary.status` is last-write-wins: a word that went new → unknown →
-- known leaves only the final stamp, so "how many words did I call known on
-- 2026-08-14" is unanswerable from it, and no later migration can reconstruct
-- it. This log is the answer, and it only ever grows.
--
-- The count at any instant is a fold over the log, not a stored total:
--   SELECT ts, SUM(CASE WHEN status = 'known' THEN 1
--                       WHEN prev   = 'known' THEN -1 ELSE 0 END)
--            OVER (ORDER BY ts) FROM vocabulary_events WHERE counted = 1;
-- At this volume (one row per assertion, ~15k after the cold start) that is a
-- few ms. A rollup table can be derived from the log later if it ever isn't;
-- nothing can be derived from a rollup.
CREATE TABLE IF NOT EXISTS vocabulary_events (
    id       INTEGER PRIMARY KEY,
    headword TEXT NOT NULL,
    reading  TEXT NOT NULL,
    ts       REAL NOT NULL,        -- epoch seconds, the assertion's own stamp
    prev     TEXT,                 -- status before; NULL = the row was created here
    status   TEXT NOT NULL,        -- status after

    -- Whether the word counted toward the vocabulary scale *at the time*
    -- (`in_master OR promoted` — see COUNTS_AS_VOCAB). Denormalized on purpose:
    -- both flags are refreshed wholesale when a dictionary is imported, and
    -- joining to them live would silently redraw the whole historical curve on
    -- the next import.
    counted  INTEGER NOT NULL,

    -- Which pass made the assertion: 'triage', 'reader', 'jiten', 'seed',
    -- 'blacklist', 'cold-start'. Informational — carried from
    -- `vocabulary.status_source` by the trigger below.
    source   TEXT,

    -- How the word had been met at the moment of the judgement. Point-in-time
    -- and unrecoverable by a later join: `lookup_count` keeps growing after the
    -- assertion. Free for the trigger to copy, so it is copied — it is the only
    -- way to ever ask whether a word was looked up before I called it known.
    first_seen     REAL,
    lookups_prior  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_vocabulary_events_ts ON vocabulary_events(ts);
CREATE INDEX IF NOT EXISTS idx_vocabulary_events_word
    ON vocabulary_events(headword, reading);

-- Logged by trigger, not by the six call sites in vocabulary.rs.
--
-- Two of those sites are set-based UPDATEs over thousands of rows with no
-- per-row Rust loop to hook (`blacklist_non_words`, the lexeme merge), and the
-- guarantee that matters is that no assertion is ever missed — including by a
-- write path that doesn't exist yet. A trigger cannot be forgotten. The cost is
-- that `source` has to travel in a column; a site that forgets to set it loses
-- a label, not an event.
DROP TRIGGER IF EXISTS vocabulary_status_insert;
CREATE TRIGGER vocabulary_status_insert AFTER INSERT ON vocabulary
WHEN NEW.status != 'new'
BEGIN
    INSERT INTO vocabulary_events (headword, reading, ts, prev, status, counted,
                                   source, first_seen, lookups_prior)
    VALUES (NEW.headword, NEW.reading,
            COALESCE(NEW.status_ts, (julianday('now') - 2440587.5) * 86400.0),
            NULL, NEW.status,
            CASE WHEN NEW.in_master = 1 OR NEW.promoted = 1 THEN 1 ELSE 0 END,
            NEW.status_source, NEW.first_seen, NEW.lookup_count);
END;

DROP TRIGGER IF EXISTS vocabulary_status_update;
CREATE TRIGGER vocabulary_status_update AFTER UPDATE OF status ON vocabulary
WHEN NEW.status != OLD.status
BEGIN
    INSERT INTO vocabulary_events (headword, reading, ts, prev, status, counted,
                                   source, first_seen, lookups_prior)
    VALUES (NEW.headword, NEW.reading,
            COALESCE(NEW.status_ts, (julianday('now') - 2440587.5) * 86400.0),
            OLD.status, NEW.status,
            CASE WHEN NEW.in_master = 1 OR NEW.promoted = 1 THEN 1 ELSE 0 END,
            NEW.status_source, NEW.first_seen, NEW.lookup_count);
END;

-- The cold start, once. Every judgement that existed before the log did, at its
-- real `status_ts` — which for the 14.5k rows of the 2026-07-27..31 triage
-- sweep is a near-vertical cliff. That is what actually happened: the sweep
-- recorded a backlog, it did not learn 14.5k words in five days. The chart
-- draws the seed as a baseline; the data stays honest about when it was
-- asserted.
INSERT INTO vocabulary_events (headword, reading, ts, prev, status, counted,
                               source, first_seen, lookups_prior)
SELECT headword, reading, status_ts, NULL, status,
       CASE WHEN in_master = 1 OR promoted = 1 THEN 1 ELSE 0 END, 'seed',
       first_seen, lookup_count
FROM vocabulary
WHERE status != 'new' AND status_ts IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM vocabulary_events);
