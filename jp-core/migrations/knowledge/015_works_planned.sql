-- The not-yet-read shelf is called planned now. queued was the same thing
-- under a name that said nothing about intent, and one shelf needs one term.
UPDATE works SET status = 'planned' WHERE status = 'queued';
