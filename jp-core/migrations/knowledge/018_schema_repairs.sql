-- How far each one-off data repair has got.
--
-- The migrations are replayed on every open and each is written to be
-- idempotent, which makes replaying them *safe* — not free. Two of them rewrite
-- data rather than declare a table, and finding nothing left to do cost half a
-- second of full table scans on every start of every tool. One of those tables
-- grows with everything the reader reads, so the cost grew with it.
--
-- `mark` is how far the repair has been applied: the last `lines.id` cleaned,
-- for a repair over a table that keeps growing. A repair that is simply finished
-- stores 0 and the row's existence is the whole answer.
CREATE TABLE IF NOT EXISTS schema_repairs (
    name TEXT PRIMARY KEY,
    mark INTEGER NOT NULL,
    ts   REAL NOT NULL
);
