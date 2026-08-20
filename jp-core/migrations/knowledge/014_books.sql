-- A physical book's text, so reading paper can be logged the way a hooked VN
-- is: exactly, and as text the ledger can be built from.
--
-- The epub of the book being read is flattened once into a single plain-text
-- string, and every position in the book is a byte offset into it. A session
-- is logged by naming where it *ended* — ten characters typed off the page —
-- which is searched forward from the last position; the span between the two
-- is what was read, and it goes into `manual_sessions.content` like any other
-- pasted text. Nothing downstream knows a book from an article.
--
-- The text is stored rather than the file path because the offsets are only
-- meaningful against one exact flattening: a re-parse under a changed stripper
-- would move every position already recorded.
--
-- `work` is the same exact title string `lines.work` and `manual_sessions.work`
-- carry, and the `works` row beside it holds the status — so a book is marked
-- finished the same way a VN is, and there is no second notion of "done" here.
CREATE TABLE IF NOT EXISTS books (
    work        TEXT PRIMARY KEY,
    text        TEXT NOT NULL,

    -- Where the story starts, from the anchor typed when the book was added.
    -- Front matter, the TOC and the copyright page are inside `text` — they
    -- have to be, or an anchor search could not be given one coordinate space
    -- — but they are before this and are never read, counted or paged.
    body_start  INTEGER NOT NULL DEFAULT 0,
    -- How far the reader has got. The next session starts here.
    position    INTEGER NOT NULL DEFAULT 0,

    -- Counted once at setup so the book list never has to load `text`, which is
    -- a megabyte per row.
    body_chars  INTEGER NOT NULL DEFAULT 0,
    text_bytes  INTEGER NOT NULL DEFAULT 0,

    -- The printed page numbers the body text runs between, read off the paper
    -- copy. Chars-per-page is derived from these rather than from a total page
    -- count, because a total counts the blanks, the TOC and the afterword and
    -- so makes every page estimate read high.
    first_page  INTEGER,
    last_page   INTEGER,

    added_ts    REAL NOT NULL
);
