-- The vocabulary ledger: one row per term, holding what is known about it.
--
-- This is the convergence layer of spec/knowledge-db.md's second axis. Every
-- other table here records an *event* — a line was read, a word was looked up,
-- a card was made. This one records a *state*: what I know. Downstream features
-- (the #read highlighter, i+1 sentence marking, "how many unknown words are in
-- this video") all reduce to an O(1) status lookup against this table, which is
-- why the counts live on the row rather than being derived per query.
--
-- It replaces the empty `vocabulary` stub that sat in yt-mine.db: that one was
-- keyed on lemma alone and carried a user_id this workspace has no use for.

-- Identity is (headword, reading), not lemma alone: 空 is そら or から, 辛い is
-- からい or つらい, and marking one known must not mark the other. Both come
-- from the tokenizer, with the reading normalized to hiragana
-- (jp_core::text::kana) so it joins dictionary_entries.reading, which is
-- hiragana too.
--
-- reading is '' for a kana-only headword: there the two strings are equal and
-- storing both would be storing one fact twice. It is never NULL, so the
-- primary key always compares.
CREATE TABLE IF NOT EXISTS vocabulary (
    headword TEXT NOT NULL,
    reading  TEXT NOT NULL,

    -- Sudachi's top-level part of speech, most recently seen. Informational —
    -- it disambiguates a homograph in the triage UI. Not part of the key: a
    -- word used as both noun and verb is one word.
    pos TEXT,

    -- What *I* have asserted, and nothing else. Never written by a sync.
    --   new         ingested from reading, never judged. The default.
    --   known       I know this word.
    --   unknown     I looked at it and I don't.
    --   learning    actively being learned (set by hand, not by the Anki sync)
    --   blacklisted never surface this again — noise, or a word I refuse to care about
    --   name        a proper noun, not vocabulary
    -- `new` is kept distinct from `unknown` deliberately: once ingest has
    -- written `unknown` across every word ever read, no later migration can
    -- reconstruct which of them I actually judged. That distinction is what
    -- makes "seen 12 times, never judged" (cold-start Pass 4) and the seed
    -- page's progress figure answerable at all.
    status TEXT NOT NULL DEFAULT 'new',
    -- When status last changed by assertion. NULL while it is still `new`.
    status_ts REAL,

    -- Anki's fact, mirrored — not a status. Recomputed wholesale from
    -- anki_notes on every refresh, exactly as that snapshot itself is, so drift
    -- is fixed by re-syncing. A freshly mined word is mined=1, status='new',
    -- which is accurate: it was mined *because* it wasn't known. Each feature
    -- picks its own rule over the pair rather than having one baked in here.
    mined INTEGER NOT NULL DEFAULT 0,

    -- Running aggregates, the reason this table exists. encounter_count is
    -- incremented by the watermarked ingest over `lines` + `manual_sessions`;
    -- lookup_count is recomputed wholesale from `lookups`, which owns it.
    encounter_count INTEGER NOT NULL DEFAULT 0,
    lookup_count    INTEGER NOT NULL DEFAULT 0,
    first_seen REAL,                 -- epoch seconds, earliest encounter
    last_seen  REAL,

    -- The dictionary gate, by role, refreshed wholesale from
    -- dictionary_entries. Stored rather than joined because the highlighter
    -- asks it per token as each line arrives, and because the answer only
    -- changes when a dictionary is imported.
    --   in_master     counts toward "I know N words" (Sankoku's ~82k scale)
    --   in_name       a name dictionary has it
    --   in_reference  any other dictionary has it
    -- Wordhood gate (is this even a word?) = any of the three. Vocabulary
    -- denominator = in_master alone. See spec/knowledge-db.md.
    in_master    INTEGER NOT NULL DEFAULT 0,
    in_name      INTEGER NOT NULL DEFAULT 0,
    in_reference INTEGER NOT NULL DEFAULT 0,

    PRIMARY KEY (headword, reading)
);

-- Triage lists sort by encounter count within a status; the highlighter looks
-- up by headword when it has no reading to offer.
CREATE INDEX IF NOT EXISTS idx_vocabulary_status ON vocabulary(status);
CREATE INDEX IF NOT EXISTS idx_vocabulary_headword ON vocabulary(headword);
CREATE INDEX IF NOT EXISTS idx_vocabulary_encounters ON vocabulary(encounter_count DESC);
