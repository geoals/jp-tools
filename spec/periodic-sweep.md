# The periodic sweep — Pass 4

**Status: not built.** Everything it needs exists; this is assembly, not new
logic. Written 2026-07-29 as a handover, so it assumes no memory of the session
that produced it.

## What it is for

The reader's own words:

> I don't want to manually triage thousands of words. I think the jiten import
> should cover a lot of ground, and then I can have a periodic thing which
> suggests candidates for marking as known — for example after a day of reading
> or since the last batch update, I will be presented with all words which were
> not marked as known but which I have read over a certain number of times, to
> add those as known. And apart from that it will come from Anki mining.

So the steady state is: **jiten seeded the bulk, mining adds as you go, and
every week or two a small batch appears already ticked and you accept it.** The
one-word-at-a-time triage queue is not the shape wanted; it exists and works,
and this replaces its role in the daily loop.

## What already exists (do not rebuild)

| piece | where | note |
|---|---|---|
| the selection rule | `vocabulary::preselects_known` | `encounter_count >= min && lookup_count == 0` |
| the queue + counts | `vocabulary::triage_queue`, `triage_pending` | filters `status='new' AND in_master=1 AND encounter_count>=?` |
| the batch write | `vocabulary::set_status_each` | mixed statuses, one transaction |
| the seed write | `vocabulary::seed_status_each` | only fills `new`; use this if a sweep should never overrule |
| `seen` count | `vocabulary::seen_count` | derived, shares the triage floor |
| the threshold | `settings.triage_min_encounters`, default 3 | persisted in `read-stats.db` |
| the UI | `read-stats/static/panels/triage.js` | the existing per-row sweep, over `/api/vocab/queue` + `/judge` |

**The two-signal rule is the whole point and must not be weakened.** A word is
ticked `known` only when it was met often enough *and was never looked up*.
Encounters alone cannot tell "read straight past it" from "looked it up twelve
times", and an unticked row writes `unknown` on submit — so a one-signal
default writes wrong assertions in bulk. The rule lives server-side because it
decides what gets written and has to be testable without a browser.

## What to build

**1. Scope the queue to "since last sweep."** The only genuinely new state.
Store a watermark in `read-stats.db` settings — a timestamp, or better a
`lines.id`, matching how the ingest sinks already watermark
(`vocab_through_line_id` and friends; see the workspace CLAUDE.md). A term
enters the batch when it crossed the encounter threshold *since* that mark, so
a word already declined does not reappear until it is read a lot more.

Note `first_seen`/`last_seen` are on the ledger row, but neither answers "when
did this cross the threshold". Either accept `last_seen > watermark` as the
approximation (simple, slightly over-inclusive) or derive from `word_days`,
which holds per-day counts and can answer it exactly. Prefer the approximation
first; the exact version is only worth it if batches come out noisy.

**2. Accept-all.** One button that writes every preselected row `known` and
leaves the rest alone. Deliberately *not* "write the unticked ones `unknown`" —
see the open question below.

**3. Advance the watermark on submit**, not on load, or an interrupted sweep
loses its batch.

**4. Surface it.** A count on `#vocab` ("142 words ready since 21 Jul") and
ideally on the dashboard, since the trigger is "after a day of reading".

## Decisions already made — do not relitigate

- **Four statuses only**: `new`, `known`, `unknown`, `blacklisted`.
  `learning` and `name` were removed 2026-07-29 (both zero rows).
- **`unknown` is the sweep's snooze.** It is what "no" writes, not a state
  anyone sets deliberately. It exists *because* of this feature: without it a
  word met often and not known returns in every batch forever.
- **`seen` is derived, never stored** (`seen_count`). Storing it would add a
  writer to a column only the reader may write, and freeze a threshold that
  should stay re-tunable over the whole history.
- **Only the reader writes `status`.** A sweep is reader-triggered, so it
  qualifies; a *scheduled* job that wrote statuses without a person accepting
  them would not, and must not be built.
- **Count words, not rows.** `lexeme::known_lexemes` collapses spellings
  (叔父/伯父/おじ = one word). Any figure this feature reports should use it, not
  a row count.
- **`vocabulary::COUNTS_AS_VOCAB`** (`in_master = 1 OR promoted = 1`) is the one
  predicate every vocabulary figure gates on. Use it; do not write
  `in_master = 1` inline.

## The data, as of 2026-07-29

| | |
|---|---|
| ledger rows | 15,945 |
| `known` / `new` / `unknown` / `blacklisted` | 13,758 / 2,151 / 35 / 1 |
| known **words** (lexeme-collapsed) | 11,694 |
| known spellings (in master) | 13,479 |
| `seen` (new, in master, ≥3 encounters) | 188 |
| promotion candidates | 351 |

**Line-level tracking begins 2026-07-19** — about 500k characters, plus 17
manual sessions back to 2025-12-29. That is why ~9,000 `known` rows have
`encounter_count = 0`: they came from the jiten, Anki and frequency imports,
not from tracked reading. Expect the sweep's batches to be *small* — 188 rows
are eligible today — and to grow slowly as reading accumulates. This is a
trickle feature, not a bulk one; the bulk was the jiten import.

## Open questions for the reader

1. **Does declining write `unknown`, or just skip?** Today's triage writes
   `unknown` for anything unticked, which is what makes the snooze work. But an
   accept-all button that silently marks the *rest* `unknown` is a bulk
   assertion nobody looked at. Suggested: accept-all writes only `known`, and
   declining is an explicit per-row action. Confirm before building.
2. **What cadence?** "After a day of reading" and "since last batch" are
   different triggers. A watermark supports both; the UI has to pick a default.
3. **Should `mined` short-circuit the rule?** A word with a card is arguably
   known regardless of lookup count. `VocabRow::is_known` already treats
   `status.is_known() || mined` as the default "reader has this word", but the
   triage preselect deliberately does not. Leave as-is unless asked.

## Gotchas

- **The htm whitespace rule.** Never let literal text and `${...}` straddle a
  line break inside an ``html`` `` template — build the string in JS. This
  silently rendered `snapshot 0 min ago` as `snapshot0 minago`.
- **Check `EXPLAIN QUERY PLAN` says SEARCH, not SCAN**, for anything touching
  `dictionary_entries`. A scanning subquery once held the write lock for six
  minutes.
- **Use `scripts/dev-instance.sh`**, never the live instance, and never
  `pkill -f read-stats` — the dev instance and live share a binary path.
- **Don't restart while a VN is being read.** Check
  `SELECT MAX(ts) FROM lines` first.
- A backup of the ledger before the 2026-07-29 repair is at
  `~/.local/share/jp-tools/vocabulary-backup-20260729-213845.sql`.

## Known adjacent defect (not this feature, but nearby)

**Pass 1 (Anki import) still creates empty-reading rows** for kanji headwords
the master dictionary does not list. `repair_empty_readings` cleans them up
(207 re-keyed, 25 merged on 2026-07-29) but the import keeps making them: it
should fall back to `dictionaries::any_readings` before storing an empty
reading, the same way the repair does. Worth fixing before the next Anki
import, and it is a ten-line change in `vocab_anki_import`.
