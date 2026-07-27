# Feeding pasted reading into the knowledge layer

Status: done (2026-07-26; re-verified 2026-07-27). Kept as the record of *why*
it is shaped this way; the rule in "The constraint that shapes the whole thing"
is the one to preserve if any of this is touched again.

One thing landed after this was written and is worth knowing here: the same
`ingest_new_sessions` pass now also feeds the **`vocabulary` ledger**, not just
`word_days`, behind a `vocab_through_session_id` watermark of its own
(`spec/knowledge-db.md`). That makes pasted text count as vocabulary
calibration — `spec/cold-start.md` Pass 2 leans on it — and it does *not*
weaken the exposure/cost rule below, because the ledger's `lookup_count` is
recomputed wholesale from `lookups`, which article reading never writes to.

A logged session can carry the text it was read from — `manual_sessions.content`,
with `url` beside it. It makes `chars` exact, counted by
`jp_core::text::chars::count_chars`, the same rule the hooked line stream is
held to, and it counts as *reading*: the kanji met in an article are kanji met,
and the words are words the reading has shown you again.

## The constraint that shapes the whole thing

Lookups are only recorded while the line stream is live. `ankiproxy::record`
drops a lookup unless a line arrived within `session_gap_secs`, so that reading
a news article never puts a term in a VN's funnel and never inflates a rate
whose denominator the line stream cannot see. **That guard stays.** Article
lookups are not captured, and are not going to be.

That decision is settled, and it is what makes the rest delicate rather than
trivial. Any rate here is `lookups ÷ encounters`. If article characters enter
the denominator while article lookups can never enter the numerator, the rate
is **diluted** in proportion to how much of the reading is articles — the
per-1000-character lookup rate on Trends being the live case.

So the rule for everything below:

> Article text feeds every count that is about **exposure**, and no count that
> is about **cost**.

This is the focus-metric bug in a new place — a measure that punishes the
reader for reading something the instrumentation cannot see — and it is worth
recognising as the same shape before writing any of it.

## Step 0 — the rate denominators (done first, separately)

`lookups_per_1k` divided by a day's *total* characters, so manually logged
reading was already diluting it before any of the below — a pre-existing bug
that articles turn from rare into routine. It now divides by hooked characters
only, in `routes/summary.rs` and `routes/days.rs`, with the rule stated in
`stats::rate`.

The same audit found a second, unrelated circularity: an estimated session's
duration is *derived* from the reader's pace, so feeding it into a speed chart
has the chart partly measuring its own output. `History::measured_days` is the
denominator every chars/hour figure now divides by — the line stream plus
manual sessions that were logged with real minutes. Totals, goals and streaks
still count everything read; only speed is restricted.

## Step 1 — tokenize content into `word_days`

`ingest.rs` tokenizes new lines behind a `tokenized_through_line_id` watermark.
Sessions get a second watermark, `tokenized_through_session_id`, and run the
same Sudachi path with the same mined-vocab validation headwords — a mined
compound found whole in Mode C stays whole, so it still matches its card.

A session's words are attributed to the day its `start_ts` falls in: one date
for the whole row. There are no per-line timestamps to spread them across, and
inventing them was already ruled out when `content` was put on the session row
rather than expanded into `lines`.

`word_days`' one consumer is the mined-word re-encounter panel — "of the words
I carded, which has the reading actually shown me again?" — which article
reading genuinely does answer. No split needed here.

## Decisions taken

**Articles collapse into one work.** Every article aggregates under
`stats::work::ARTICLES_WORK`. An article is a source but not a *work* in the
sense that view is about — a thing read through, with a cover, a status and a
queue position — and thirty of them would bury the four VNs the list exists
for. The individual title and URL stay on the session row and show in the day's
sittings table, which is where "what did I read on Tuesday" is asked.

**`word_days` is not split.** Reading an article genuinely re-showed you the
word, which is exactly what the re-encounter panel asks.

## What it actually moved

Behaviour-changing by design, so `dev-instance.sh check` does not print
IDENTICAL — the diff *is* the review. The `pre-articles` snapshot was taken
first and is the record of what every rate was before articles entered any
denominator; the raw-speed / lookup-tax study (~Aug 2026) will want it.

Against the real database, one logged article (2,459 chars):

- `total_encounters` 51,398 → 51,898: 233 kanji gained exposure, 4 were met for
  the first time ever.
- `works` gained one `Articles` row instead of one row per headline.
- today's `lookups_per_1k` went from 0.41 to `null` — it had been one stray
  lookup divided by article characters, which is exactly the number the rule
  above exists to refuse.
