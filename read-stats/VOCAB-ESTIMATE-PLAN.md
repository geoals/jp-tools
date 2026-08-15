# Estimated vocabulary size, tracked day to day

A panel that answers "how many words do I know" from reading behaviour rather
than from what has been judged, and plots it over time.

The ledger already answers a narrower question — 10,190 master terms asserted
`known` — but that only counts words *met and judged* inside a four-week,
664k-character corpus. It is a floor that moves when marking discipline moves,
which is the wrong property for a number meant to show growth.

This estimates the real figure instead, from the one signal that is independent
of marking: **what was read and what was looked up.**

---

## The measurement it rests on

Run 2026-08-15 over a 30-minute session (290 lines, 4,860 chars, 2,715 word
tokens, 747 distinct), with every unknown word deliberately looked up, then
widened to the surrounding two days.

The ground truth is one rule: **a word looked up was not known; a word read
without a lookup was.**

What it produced, over the same tokens:

| | by word | by text |
| --- | --- | --- |
| ledger's claim | 86.5% | 94.3% |
| actual | 97.5% | 99.3% |

The error is not spread out. Assertions held at 99% (4 wrong of 642, and 2 of
those 4 were tokenizer artefacts). **The whole gap is the unjudged bucket being
scored as zero when 84% of it is known.**

Projected onto the master's headwords by frequency band, that puts vocabulary at
roughly **18,000 inside Jiten's top 30,000**, against 20,311 master terms in
that range. Two facts about that number matter more than the number:

- **±2,000 is the honest span.** Not the ±650 a bootstrap reports.
- Two adjacent days of the same work, fitted separately, gave **17,545 and
  18,799**. Out-of-sample variance is twice sampling variance. Anything built
  here has to be built around that, not in spite of it.

---

## Method

### 1. The sample

Every distinct word met in a trailing window, and whether it was looked up.

```sql
-- types met
SELECT DISTINCT lemma FROM word_days WHERE date >= date('now','-30 day');
-- types looked up
SELECT DISTINCT headword FROM lookups WHERE ts >= strftime('%s','now') - 30*86400;
```

**Join on `lookups.headword`, never `lookups.term`.** That column exists because
Yomitan sends the spelling the page used and the ledger keys on Sudachi's
normalized form — see CLAUDE.md's rule on spellings from outside. `word_days.lemma`
is written by ingest and carries the same normalization, so the two join.

**Nothing needs re-tokenizing.** `word_days(lemma, date, count)` already holds
types-met-per-day, accumulated by ingest. The analysis above re-ran 1,321 lines
through `/api/tokenize` only because it needed per-token detail that `word_days`
does not keep; the estimate does not.

Caveat `word_days` does not solve: it is keyed on lemma alone, with no reading
and no part of speech, so grammar words (の, だ, た) are in it. They rank in the
top few hundred where the known-rate is ~100% regardless, so the effect is
negligible — but a rate computed *per band* must not be told that band 1 is
90% particles. Filter by joining `vocabulary.pos`, or accept it and say so.

### 2. Window

**30 days, recomputed daily.** Not one day.

| window | types met | of those, ranked ≤30k |
| --- | --- | --- |
| 7 days | 7,461 | — |
| 14 days | 9,138 | — |
| 30 days | 13,067 | **10,167** |

At 30 days you **directly observe half the ≤30k block**. That turns the estimate
from mostly-model into half-counting, and it averages out the day-to-day swing
that dominates the single-day fits. It also spans several works, which is the
only defence against the estimate tracking what is being read.

### 3. Two bounds, both mechanical

- **lower** — every lookup counts as not-known.
- **upper** — lookups on terms already asserted `known` in the ledger do not
  count.

The upper rule exists because those lookups are second-guesses, not gaps: 方, 性
and 塵 in the audited session were all already-`known` rows. It also discards
tokenizer artefacts for free — 方 was a lookup on かたや mis-segmented as 方/かた,
and the rule drops it without any special-casing.

Serve both. The gap between them is the reader's own second-guessing rate, which
is worth seeing.

### 4. Rate curve

Bin by Jiten rank, take the observed known-rate per bin, apply it to the master
headwords in that bin, sum.

**Do not fit a logistic in log-rank.** It was tried. It fits the head well and
its tail is badly optimistic — 42% known at rank 100k+, which is nonsense for
Sankoku's archaic tail — and it reads 2–4% high even inside the measured range.
Binning is insensitive to bin count (4, 6 or 8 bins move the total by under 30
words) and carries no shape assumption.

**Cumulate only as far as the window supports.** Require a minimum observation
count per bin, and stop at the last bin that meets it. Headline is "words known
within Jiten's top N", with N derived from the data rather than fixed.

### 5. Denominator — use lexemes, not terms

`jp_core::knowledge::lexeme` exists precisely for this: 叔父, 伯父 and おじ are
three master terms and one word.

Measured crudely on Jitendex sequence ids alone, the ≤30k block collapses
**20,311 terms → 19,291 lexemes, about 5%.** The real module also folds kana
forms onto their kanji, so the true collapse is larger.

**This is a bigger correction than the sampling error, and the analysis above did
not apply it** — every figure in this document is on the raw-term denominator and
runs about 5% high. Route the denominator through `known_lexemes`' unit and the
headline drops to roughly 17,000.

### 6. Cost

All SQL plus a few thousand rows of arithmetic. The only heavy input is the
82k-row master→rank map, which changes only when a dictionary is imported —
precompute it at `jp-dict sync` time. Daily recompute is milliseconds.

Per CLAUDE.md's lock rule: check `EXPLAIN QUERY PLAN` says SEARCH before adding
anything that touches `dictionary_entries`.

---

## Validate before building the UI

**One assumption carries the whole design: that the known-rate *within a rank
band* is stable across texts.** A hard work has more rare words, but the rate
inside a band should not move. If it does, the panel plots what is being read,
not what is known, and it is not worth serving.

This has not been tested. There was only one work to test it on.

The test: compute the per-band curve separately for two works of different
difficulty read in the same period, and check the curves coincide. Write it as a
`read-stats/examples/` binary that prints the table — the same shape as
`rebuild_preview.rs` — and read the output before committing to a panel, a route
or a daily job.

Known in advance: the whole-work curve (66% at 15–30k) and the 30-minute curve
(84%) disagree sharply, but across three days of active learning, not across
difficulty. A trailing window handles that. It is not evidence either way on the
assumption above.

---

## What it cannot do, and must therefore say

**It measures comprehension, not knowledge.** 声変わり was understood from
context having never been seen; 好敵手 was understood because the voice actor
read it as ライバル; 座付き came from exposure earlier in the same work. Nothing
in the data separates those from knowing a word. In the audited session the two
definitions differed by 2.6%, and that gap will widen on
transparent-compound-heavy material. Label the number **comprehension**.

**Lookups undercount ignorance.** A word read wrong with confidence is never
queried. 店番 was read てんばん and scored known.

**Text-sampling bias inflates within every band.** A rank-25,000 word that turns
up in a VN sits at the frequent end of its band. The model over-predicted at
rank 35k (77% against 64% observed), which is this bias showing.

All three push the same way. **The true number is more likely below the estimate
than above it**, and the panel should not present a symmetric error bar.

**Past rank ~70,000 there is nothing.** 44,365 master headwords — archaic,
technical, literary — that fiction reading will never sample. Do not extrapolate
into them. "Total vocabulary" is not a question this data can answer; "vocabulary
within the common N" is.

---

## Build order

1. `read-stats/examples/vocab_estimate.rs` — prints the bound pair, the per-band
   table, and the same numbers computed per work. **Run the per-work comparison
   and read it.** Stop here if the curves diverge.
2. Move the computation into `read-stats/src/services/` once the shape is
   settled. Query helpers stay in read-stats until a second consumer appears,
   per CLAUDE.md.
3. Persist one row per day: date, window length, lower, upper, bins observed,
   N reached, types met. **Store the inputs alongside the output** — a curve
   recomputed later against a changed dictionary set will not reproduce, and a
   growth chart whose history silently re-derives is worthless.
4. `GET /api/vocab/estimate` — current figure, and the stored series.
5. A panel under `read-stats/static/panels/`. Registration is an import plus a
   tab entry in `static/app.js`, alongside `trends`.
6. Daily recompute. `main.rs` already spawns best-effort background work
   (`backfill_sequences`, `covers::reconcile_missing`); this is the same shape.

Watch the htm whitespace rule from CLAUDE.md when building the panel's labels —
build interpolated strings in JS, never straddle a line break inside a template.

---

## What the panel should show

- **The bound pair, not a single number.** "17,000–19,000 words within Jiten's
  top 30,000" is the honest headline.
- **The observed floor, separately.** 7,209 head-block words met and never once
  looked up is a hard count with no model in it, and it is the most defensible
  number on the page.
- **The series over time**, which is the point of the feature. Day-to-day
  movement will be noise; the trend over months will not be.
- **How much of the block is observed rather than modelled** (10,167 of 20,311
  at 30 days). It tells the reader how much to trust that day's figure, and it
  rises as reading accumulates.

One thing worth designing around: the figure drifts upward as a work gets easier
partway through. Showing per-work alongside the global number makes that visible
rather than mysterious.

---

## Numbers this document asserts, and where they came from

All from the 2026-08-15 session and the two days around it, against
`knowledge.db` as of that date. All on the **raw-term denominator** — see §5.

| | |
| --- | --- |
| session | 290 lines, 4,860 chars, 2,715 word tokens, 747 distinct types |
| lookups | 26 raw, 19 distinct genuinely-unknown words after artefacts |
| comprehension | 97.5% by word, 99.3% by text |
| unjudged bucket actually known | 84% |
| master headwords | 81,884 (71,667 ranked by Jiten, 10,217 unranked) |
| master headwords ranked ≤30k | 20,311 terms / 19,291 lexemes |
| estimate ≤30k | ~18,000, span ~16,500–19,500 |
| single-day fits, Aug 14 vs Aug 15 | 17,545 vs 18,799 |
| directly observed | 9,130 head-block terms met, 7,209 never looked up |
