-- Strip the VN's own control characters out of stored dialogue.
--
-- Subahibi marks up its script with C0 control codes — \x05 heads a narration
-- line, \x04 sits mid-clause — and Textractor hands them over with the text.
-- vn-ws-logger.py deliberately does not *drop* a line for containing them,
-- since they say nothing about whether it is real reading, but it had no reason
-- to keep the bytes either: Sudachi analyses them as words, and 116 sightings
-- of "e" and 40 of "d" reached the vocabulary ledger that way.
--
-- Not a character count change. Both counters are allowlists over Japanese
-- (`charcount.rs`, and NOT_COUNTED in the logger), so a control code never
-- counted toward chars read and removing it moves no reading statistic.
--
-- Tab, newline and carriage return are left alone. Nothing else in 0x01–0x1F
-- belongs in dialogue.
--
-- **Bounded by a watermark, not idempotent-and-replayed.** Lines keep arriving,
-- so this cannot be a one-off repair — but testing every row ever captured
-- against 28 nested `replace()` calls cost a quarter of a second on every open
-- of the database, by every tool, and grew with everything read. `id` is a rowid
-- alias and nothing deletes a line, so the last id cleaned is a safe floor. The
-- GLOB is what decides whether a row needs the rewrite at all; the `replace`
-- chain then only runs on rows that do.
UPDATE lines
   SET text = replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(text, char(1), ''), char(2), ''), char(3), ''), char(4), ''), char(5), ''), char(6), ''), char(7), ''), char(8), ''), char(11), ''), char(12), ''), char(14), ''), char(15), ''), char(16), ''), char(17), ''), char(18), ''), char(19), ''), char(20), ''), char(21), ''), char(22), ''), char(23), ''), char(24), ''), char(25), ''), char(26), ''), char(27), ''), char(28), ''), char(29), ''), char(30), ''), char(31), '')
 WHERE id > COALESCE((SELECT mark FROM schema_repairs WHERE name = 'strip_control_chars'), 0)
   AND text GLOB ('*[' || char(1) || '-' || char(8) || char(11) || char(12)
                       || char(14) || '-' || char(31) || ']*');

INSERT INTO schema_repairs (name, mark, ts)
VALUES ('strip_control_chars', COALESCE((SELECT MAX(id) FROM lines), 0), strftime('%s', 'now'))
    ON CONFLICT(name) DO UPDATE SET mark = excluded.mark, ts = excluded.ts;
