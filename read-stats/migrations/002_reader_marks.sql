-- Presence marks left by engagement actions on the #read page: the explain
-- button and the mine button. These prove the reader was at the keyboard at a
-- moment the line stream can't show on its own — reading an explanation, or
-- mining without a fresh Yomitan lookup in the gap.
--
-- Only *engagement* actions land here. Suppress actions (clear, pause) never
-- do: they exist to stop counting a span, so a mark would re-credit what they
-- remove.
--
-- Merged with lookup and card timestamps into the one evidence stream
-- stats::Presence credits gap time against (see api.rs presence_marks). Kept in
-- their own table rather than written into `lookups` so they never inflate the
-- lookups/h or unknown-word-rate metrics, which count word lookups only.
CREATE TABLE IF NOT EXISTS reader_marks (
    id   INTEGER PRIMARY KEY,
    ts   REAL NOT NULL,          -- epoch seconds
    kind TEXT NOT NULL           -- explain | mine
);
CREATE INDEX IF NOT EXISTS idx_reader_marks_ts ON reader_marks(ts);
