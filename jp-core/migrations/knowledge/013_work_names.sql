-- The cast of a work, so the tokenizer can be told rather than left to infer.
--
-- The name filter can only ask Sudachi's 固有名詞 tag, and Sudachi does not
-- know a VN's cast: it misses from 0% to 47% of the time, and where it has no
-- entry for the name at all the name is not merely mistagged but *split* —
-- 世凪 into 世 + 凪, crediting 2,385 sightings of a word the text never used.
-- No threshold fixes that, because the evidence is not there to threshold.
--
-- But the cast is knowable before a word of the work is read: VNDB lists it.
-- So this is ground truth imported once per work, not a rule inferred from
-- three works of encounters.
--
-- Per work rather than global because a name in one work is a word in another:
-- 凪, 凛, 出雲 and すもも are all ordinary vocabulary somewhere. `work` is the
-- same exact title `lines.work` carries.
--
-- `source` records where the name came from ('vndb', 'manual'), so a refetch
-- can replace what it imported without touching names added by hand.
CREATE TABLE IF NOT EXISTS work_names (
    work   TEXT NOT NULL,
    -- As the text would write it: 世凪, カンナ, オリーヴ. One row per form,
    -- including each part of a full name and every alias, since a script uses
    -- whichever it likes.
    name   TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'vndb',

    PRIMARY KEY (work, name)
);

CREATE INDEX IF NOT EXISTS idx_work_names_work ON work_names(work);
