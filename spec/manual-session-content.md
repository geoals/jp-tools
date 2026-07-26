# Feeding pasted reading into the knowledge layer

Status: done (2026-07-26). Kept as the record of *why* it is shaped this way;
the rule in "The constraint that shapes the whole thing" is the one to preserve
if any of this is touched again.

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

That decision is settled, and it is what makes phase 2 delicate rather than
trivial. The kanji tab's lookup rate is `lookups ÷ encounters`. If article
characters enter `encounters` while article lookups can never enter `lookups`,
every kanji met in an article has its rate **diluted** — denominator grows,
numerator cannot. The red outlier rings would quietly stop marking the kanji
that actually cost you, in proportion to how much of the reading is articles.
The same applies to the per-1000-character lookup rate on Trends.

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

## Step 2 — split the kanji denominator

`stats/kanji.rs` builds the whole tab from one pass over raw text. `KanjiLine`
gains a flag for whether its lookups are observed (call it `metered`: true for
hooked lines, false for pasted content), and `KanjiRow` carries both `count`
(every encounter) and `metered_count`.

Then, per figure:

| figure | denominator | why |
|---|---|---|
| grid tint, `SOLID_ENCOUNTERS`, days-seen | `count` | asks how much you have met it; an article is reading |
| discovery curve, grade + corpus coverage | `count` | same — first sighting is a first sighting |
| lookup rate, `OUTLIER_ENCOUNTERS` floor, red rings, hardest-kanji ranking | `metered_count` | asks what it cost; articles cannot answer |

That split belongs in `stats/kanji.rs` and not in the SQL. It is a decision
about what a number means, which is that layer's stated job, and it is the kind
of decision that has to be unit-testable without a server.

The client had its own copies of that division — the grid's lookup-rate sort,
the inspector's percentage, the hardest-kanji ranking — and every one of them
moved to `metered_count` too. A rate computed in two places is a rate that will
eventually disagree with itself.

## Step 3 — say so

The kanji legend states the threshold it used, and now also that the readings
it counted were hooked ones. A number whose denominator differs from the one
sitting next to it has to admit that, or the page is quietly lying about which
corpus it measured.

## Decisions taken

**Per-work fingerprints.** Every article collapses into one synthetic work,
`stats::work::ARTICLES_WORK`. An article is a source but not a *work* in the
sense that view is about — a thing read through, with a cover, a status and a
queue position — and thirty of them would bury the four VNs the list exists
for, each carrying a fingerprint built from two thousand characters. Collapsed,
they make one fingerprint worth reading: what article reading looks like beside
fiction. The individual title and URL stay on the session row and show in the
day's sittings table, which is where "what did I read on Tuesday" is asked.

**`word_days` is not split.** Reading an article genuinely re-showed you the
word, which is exactly what the re-encounter panel asks. It does mean that
panel and the kanji lookup rate count different corpora — the one thing to stay
alert to here, and worth revisiting if the panel ever grows a rate of its own.

## What it actually moved

Behaviour-changing by design, so `dev-instance.sh check` does not print
IDENTICAL — the diff *is* the review. The `pre-articles` snapshot was taken
first and is the record of what every rate was before articles entered any
denominator; the raw-speed / lookup-tax study (~Aug 2026) will want it.

Against the real database, one logged article (2,459 chars):

- `total_encounters` 51,398 → 51,898; `total_metered_encounters` stays 51,398.
  233 kanji gained exposure, 4 were met for the first time ever — present in
  the grid and the coverage curves, and unrankable by cost, which is correct:
  nothing was measured about what they cost.
- every lookup rate, the baseline and all 36 red rings: **unchanged**, which is
  the whole point of the split.
- `works` gained one `Articles` row instead of one row per headline.
- today's `lookups_per_1k` went from 0.41 to `null` — it had been one stray
  lookup divided by article characters, which is exactly the number the rule
  above exists to refuse.
