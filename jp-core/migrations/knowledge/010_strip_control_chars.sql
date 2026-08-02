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
-- belongs in dialogue. Idempotent by construction, and the WHERE clause means a
-- replay after the first touches no rows at all.
UPDATE lines
   SET text = replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(text, char(1), ''), char(2), ''), char(3), ''), char(4), ''), char(5), ''), char(6), ''), char(7), ''), char(8), ''), char(11), ''), char(12), ''), char(14), ''), char(15), ''), char(16), ''), char(17), ''), char(18), ''), char(19), ''), char(20), ''), char(21), ''), char(22), ''), char(23), ''), char(24), ''), char(25), ''), char(26), ''), char(27), ''), char(28), ''), char(29), ''), char(30), ''), char(31), '')
 WHERE text <> replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(text, char(1), ''), char(2), ''), char(3), ''), char(4), ''), char(5), ''), char(6), ''), char(7), ''), char(8), ''), char(11), ''), char(12), ''), char(14), ''), char(15), ''), char(16), ''), char(17), ''), char(18), ''), char(19), ''), char(20), ''), char(21), ''), char(22), ''), char(23), ''), char(24), ''), char(25), ''), char(26), ''), char(27), ''), char(28), ''), char(29), ''), char(30), ''), char(31), '');
