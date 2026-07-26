# Feeding pasted reading into the knowledge layer

Status: phase 1 done (2026-07-26), phase 2 planned.

A logged session can carry the text it was read from — `manual_sessions.content`,
with `url` beside it. Today that text does exactly one job: it makes `chars`
exact, counted by `jp_core::text::chars::count_chars`, the same rule the hooked
line stream is held to. Nothing else reads it.

Phase 2 is making it count as *reading*: the kanji you met in an article are
kanji you met, and the words are words the reading has shown you again.

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

## Step 3 — the same audit everywhere a rate has a character denominator

The per-1000-character lookup rate needs its denominator to stay hooked
characters. Rather than fixing the two instances that are already known, walk
every consumer of a character total and ask which of the two questions it is
asking. The answer is mechanical once the rule above is stated; the risk is
missing one, not getting one wrong.

## Step 4 — say so

The kanji legend already states the threshold it used. It should also state
that the rate is over hooked reading only. A number whose denominator differs
from the one sitting next to it has to admit that, or the page is quietly
lying about which corpus it measured.

## Open decisions

**Per-work fingerprints.** `manual_sessions.work` joins `works` the same way
`lines.work` does, so every article title becomes a work with its own kanji
fingerprint. Thirty NHK articles would bury a VN in that list, and
`FINGERPRINT_FLOOR` will not save it — a short article easily hits five
occurrences of some kanji. Options: exclude `source = 'article'` from
fingerprints; collapse all articles into one synthetic work; or accept a long
list. Undecided.

**Whether `word_days` should be split too.** Step 1 says no. But it means the
re-encounter panel and the kanji lookup rate count different corpora, and this
codebase has been bitten before by two views deriving the same idea
differently. Worth revisiting if the panel ever grows a rate.

## Verifying it

This is behaviour-changing by design, so `dev-instance.sh check` will not print
IDENTICAL — the diff *is* the review, and it should be read rather than
skimmed. Take the snapshot **before** starting: it is the only record of what
every historical rate was before articles entered the denominators, and the
planned raw-speed / lookup-tax study (~Aug 2026) will want the before-picture.
