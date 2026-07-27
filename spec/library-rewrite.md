# The Library page, rewritten around the work

Status: in progress (2026-07-27). Phases 1, 2, 3, 6 and 7 are **built**. Phase
5 is next; phase 4 is approved but deferred.

The Library was one flat table: a row per work with chars, time, speed and
dates, and nowhere to put a seventh column. What jpdb and jiten do — and what
this wants — is list → drill-down, with the per-work numbers living on a page
of their own.

Two rules the whole rewrite is held to:

> **A work with no reading behind it does not appear.** No queue, no status
> lanes, no pre-added titles sitting empty. There is no text until it has been
> read, so every figure here would be blank for them.

> **Anything about a work is derived from that work's slice of the line
> stream.** Same shape as the rest of read-stats: nothing precomputed that a
> query can re-derive under a changed threshold.

## Phase 1 — list and detail (built)

`works-shelf.js` replaces the old `works-table.js`: cards with cover, title,
progress against `total_chars` when set, chars, time, speed. Clicking one
replaces the whole tab with `work-detail.js` rather than expanding a row.

**The open work lives in the URL** (`#library/<percent-encoded title>`), not in
a `useState`. Opening a work is a navigation: back returns to the shelf instead
of leaving the tab, and a work's page can be linked and reloaded.
`encodeURIComponent` escapes `/` too, so splitting the hash on it can never cut
a title in half.

**Keyed by title, not id.** `GET /api/works/detail?work=<title>` — title is the
join key lines are stamped with, nothing upserts a `works` row for a title you
start reading, and the synthetic `Articles` work can never have one. Metadata
is optional on that endpoint; the reading is what makes a work real here.

The vocabulary, dialogue and log-form cards stay at the list level, untouched:
they are about the reading as a whole, not about any one work.

`dev-instance.sh browser` now renders `#library` too. The detail panel is
reached only by clicking a card, so a bad import there would leave the tab
blank while every JSON endpoint still passed — the same trap the kanji check
exists for.

## Phase 2 — the work's own reading history (built)

The first section of the detail page, all of it from the work's lines and its
manual sessions:

- daily chars, scoped to the work's own reading days rather than a global
  window — four sittings over two weeks looks nothing like an hour a night;
- the sittings themselves: date, duration, chars, chars/hour;
- pace per sitting, so a work getting easier as its vocabulary settles is
  visible rather than remembered;
- progress against `total_chars`, and time remaining at *this work's* pace.

A manually logged work has sessions but no lines, so its bars degenerate to one
per session. That is thinner, not broken.

Sittings come from `derive_sessions` over the work's own lines, so an interlude
in another VN reads as a gap and closes the sitting — which is what it was.
Speed is blank below ten minutes and on any session whose duration was
*estimated*: that duration came from the reader's pace, so it can only report
it back.

**`aggregate_works` now credits through `Presence`.** It had its own
`min(gap, afk_secs)`, which put the shelf and the detail page 0.2% apart on the
same work (243,079 s against 242,517 s) — two answers to "how many hours is
this" one click from each other, which is what the one-rule invariant exists to
prevent. `WorkLine` therefore carries the whole `LineEvent` rather than a
`(ts, chars)` pair, since pricing a gap needs the line it follows. The two
surfaces now return the same seconds and the same chars/hour by construction.
`db::fetch_work_lines` went with it — `History::work_lines()` had superseded it
and nothing called it.

## Phase 7 — the shelf (built)

- hours remaining on what is being read now, at *this work's* own speed rather
  than the reader's average — a harder VN says so in its own estimate;
- finished works as a shelf of covers, with chars and dates on hover.

**A card is one control.** No buttons inside it: a button within a button is
ambiguous to a mouse and broken to a keyboard. "Read this" — pointing the
logger at a title — moved onto the work's own page, and is hidden for
`Articles`, which is a bucket for text logged after the fact and not something
a hooker can ever stamp a line with.

No queue and no status lanes: reading and finished are the states with data.
`queue_pos` stays in the schema because it is already there.

## Phase 6 — what the prose is like (built)

`stats/prose.rs`, one compact card: the 「」 share, the median sentence and its
90th percentile, and the median split by register — each stated as a ratio
against everything *else* read, since a bare "38% dialogue" is unreadable
without knowing the corpus runs at 52%. Both sides come out of one pass over
the text, which is why the endpoint asks for the whole stream and partitions it
rather than querying one work.

**Not every work brackets its dialogue.** Under `BRACKET_FLOOR` (10% of
characters) the split is omitted rather than reported as 100% narration — that
would be a measurement of the punctuation dressed up as one of the writing.
Ten rather than a token two percent because there is nothing in between worth
reporting: a work that brackets its speech runs far above it (素晴らしき日々 is
at 69%) and one that only brackets the occasional shout or title sits in the
low single digits (魔法少女ノ魔女裁判, 1.0%).

Percentiles need `SENTENCE_FLOOR` (200) sentences or they are `None` — over
forty sentences a median describes one scene, and printing it beside a corpus
average invites reading noise as a difference.

**Only captured text can be measured.** 素晴らしき日々 has 771,732 characters
on the shelf and 101,725 of them as text: sixteen sessions were logged by hand,
as a character count with no text behind it. The card says so whenever coverage
falls under 90%, because the alternative is a figure that looks like it
describes the work and describes an eighth of it.

## Phase 3 — per-work vocabulary (built)

`work_terms` (migration 006), keyed `(headword, reading, work)` like the ledger
so the join to it is exact — `word_days`' lemma key would let 辛い borrow its
homograph's status. Filled by the same Sudachi pass behind its own pair of
watermarks (`work_terms_through_line_id` / `_session_id`), so the backfill runs
through `POST /api/vocab/rebuild` without double-counting the other two sinks.
Queries live in `jp_core::knowledge::work_terms`.

**Both coverage figures, never one.** By type — of the distinct words here, how
many are known — is how much studying it needs. By token — of the running text
— is how it will feel to read. I expected ten points apart; against the real
data it is closer to forty-five:

| work | types | known by type | known by text |
|---|---|---|---|
| 素晴らしき日々 | 3,256 | 31% | 78% |
| 魔法少女ノ魔女裁判 | 3,583 | 32% | 78% |
| ドーナドーナ | 1,653 | 35% | 60% |

Reporting either alone would answer a question nobody asked.

Only words a dictionary recognizes are counted, on the same lenient gate the
triage queue uses — the tokenizer's fragment tail would otherwise put a floor
under every "unknown" figure that has nothing to do with the work.

**Names are not vocabulary.** A VN's cast were the top of every unknown list
(ノア×194, レイア×191), which is a cast list wearing a study plan's clothes.
`in_name` was no help — no name dictionary is loaded, so it is 0 for
everything — but Sudachi's part-of-speech *subclass* (固有名詞) was being
discarded one field away from where it was needed. `Token` now carries it.

The verdict is per **term**, over the whole pass, not per occurrence: Sudachi
tags a given surface as 固有名詞 only some of the time, and filtering occurrence
by occurrence left ノア with 79 of its 194 — worse than either whole answer. A
majority of a term's own occurrences settles it, which drops ノア and レイア
entirely while keeping 空, 光 and 時, tagged as names once in a hundred
sightings and vocabulary the rest of the time.

Names are dropped from the ledger and `work_terms`, kept in `word_days`: that
sink asks what text was read, and a name on the page is text. All of it is
re-derivable, so the rule is reversible by a rebuild.

`POST /api/vocab/rebuild` now ends with `prune_untouched`, which deletes ledger
rows the re-ingest left on zero encounters — a name under the new rule, or a
term a re-tokenization splits differently. It spares anything judged or mined:
シェリー stays at zero encounters because the reader marked it known.

**One limit left as it is:**

- **117 headwords still carry more than one reading** after the tokenizer fix.
  Some are genuine homographs and must stay split (入る/いる is not
  入る/はいる). Others are one word the dictionary lists twice — 言う as both
  いう and ゆう — which splits its counts and its judgement across two rows.
  Telling the two cases apart needs knowledge neither the ledger nor Sudachi
  offers, so nothing is merged automatically.

## Phase 5 — what the work gave you

The reverse of phase 3, and free: note ids are epoch milliseconds and lookups
are timestamped, so both attribute to whatever was being read at that moment.

- cards mined from this work;
- words first met here that are now `known` — the closest thing to a per-work
  learning outcome;
- cards added per day, beside the chars bars from phase 2: mining rate falling
  as a work goes on is the work getting easier.

A card added while nothing was hooked has no work and lands unattributed rather
than being guessed at.

"Looked up here but never carded" was proposed and cut.

## Phase 4 — difficulty (deferred)

Two measures, side by side on the detail page and sortable on the list:

- **text difficulty**, a property of the writing — share of tokens outside the
  common frequency core, share of non-jōyō kanji, mean sentence length. Stable:
  it does not move as the reader improves;
- **measured cost** — lookups per 1000 chars while reading it, and chars/hour
  against baseline pace. This one does move, which is the point.

Then every work on one chart, one against the other. The residual is where
engagement hides: a work read faster than its prose predicts.

Neither exists for a work with no reading behind it, and measured cost needs
enough hooked reading to be a rate at all — twenty minutes gets no number
rather than a noisy one.

## Rejected

Per-work kanji (distinct, non-jōyō share, first-met-here) — too close to the
per-work fingerprints deleted the same day, and countable is not actionable.
Ratings and free-text notes. Full-text search of a work's lines.

## Progress

- [x] Phase 1 — list and detail
- [x] Phase 2 — reading history
- [x] Phase 7 — the shelf
- [x] Phase 6 — prose character
- [x] Phase 3 — per-work vocabulary
- [ ] Phase 5 — what the work gave you
- [ ] Phase 4 — difficulty (deferred)
