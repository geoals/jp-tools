-- Drop Sankoku's reading-boundary marker from headwords already imported.
--
-- See `strip_okurigana_marker` in jp_core::dictionary, which keeps it out of
-- new imports: ＝ marks where a headword's reading divides (味＝方 = み + かた),
-- so a term keyed with it matches no token and no ledger row. 味方 had 11
-- encounters and in_master = 0.
--
-- The GLOB skips katakana terms, where ＝ separates two names and belongs to
-- the word — Jitendex's サピア＝ウォーフの仮説. Idempotent: the WHERE clause
-- finds nothing on a replay. `dictionary_entries` has no unique key, so a strip
-- that collides with a plainly-spelled row simply leaves both.
--
-- The ledger flags fix themselves: `refresh_dictionary_flags` recomputes
-- in_master wholesale from this table and runs on every ingest.
UPDATE dictionary_entries
   SET term = replace(term, '＝', '')
 WHERE term LIKE '%＝%'
   AND term NOT GLOB '*[ァ-ヺ]*';

-- The marker reached the ledger as well, through the triage pass that seeds a
-- row straight off a master headword rather than off a token: 不貞＝腐れる is
-- keyed with it, and the word it stands for can never be met again under that
-- key. Same three tables the key spans.
--
-- Guarded rather than merged, since no plain row collides with one of these
-- today — a collision would need the aggregates added up, and writing that for
-- a case that does not exist is how it goes wrong when it finally does.
UPDATE vocabulary
   SET headword = replace(headword, '＝', '')
 WHERE headword LIKE '%＝%'
   AND headword NOT GLOB '*[ァ-ヺ]*'
   AND NOT EXISTS (SELECT 1 FROM vocabulary w
                    WHERE w.headword = replace(vocabulary.headword, '＝', '')
                      AND w.reading = vocabulary.reading);

UPDATE term_surfaces
   SET headword = replace(headword, '＝', '')
 WHERE headword LIKE '%＝%'
   AND headword NOT GLOB '*[ァ-ヺ]*'
   AND NOT EXISTS (SELECT 1 FROM term_surfaces w
                    WHERE w.headword = replace(term_surfaces.headword, '＝', '')
                      AND w.reading = term_surfaces.reading
                      AND w.surface = term_surfaces.surface);

UPDATE vocabulary_events
   SET headword = replace(headword, '＝', '')
 WHERE headword LIKE '%＝%'
   AND headword NOT GLOB '*[ァ-ヺ]*';
