# Cold Start — Bootstrapping the Knowledge Base

This is the most critical problem to solve. Every downstream feature (highlighting,
i+1 filtering, card mining) depends on an accurate vocabulary database. The goal
is to go from zero to a reasonable approximation of your actual knowledge quickly.

> **Status (2026-07-27).** The ledger these passes fill now exists and holds
> **7,949 terms** backfilled from the whole reading history
> (`spec/knowledge-db.md`, migration note 4). Every one of them is `status =
> 'new'`: nothing has been asserted yet, because no pass below is built. The
> plumbing is done; the passes are the work.
>
> **Update 2026-07-27: Pass 2's ledger half is built** — read-stats' `#vocab`
> tab, over `GET /api/vocab/queue` + `POST /api/vocab/judge`. It is the first
> thing in the workspace that writes `status` at all. What it covers and what it
> does not:
>
> | | |
> |---|---|
> | ✅ built | triage of terms already in the ledger, ticked by the preselect rule below; bulk-blacklist of the non-word tail; the status counts as a progress figure |
> | ❌ not built | Pass 1 (Anki import), Pass 3 (frequency list), the seed importer for epubs finished before tracking, Pass 4's periodic re-surfacing |
>
> The preselect rule, which the rest of this document should be read against:
> a word is ticked `known` only when it was met at least
> `settings.triage_min_encounters` times **and was never looked up**. The floor
> defaults to 3 — deliberately low, because the zero-lookup half is doing the
> real work. `spec/knowledge-db.md` migration note 5 has the reasoning.
>
> What the backfill measured, which changes the plan below:
>
> | | count |
> |---|---|
> | ledger terms (distinct `(headword, reading)`) | 7,949 |
> | of those, in the master dictionary — i.e. vocabulary | 6,347 |
> | headwords carrying more than one reading | 667 |
> | Anki notes | 1,995 |
> | Anki notes matching a ledger row | 431 |
> | terms encountered ≥10 times | ~1,000 |
> | terms encountered ≥5 times | ~1,800 |
>
> Two of those numbers redirect the passes. **431 of 1,995** says Pass 1 is not
> a subset of Pass 2 and has to import the deck directly. **6,347 vs a passive
> vocabulary estimated near 20k** says passes 1–2 cannot get there on their own
> and Pass 3 is load-bearing, not a top-up.
>
> **Update 2026-07-28: Passes 1 and 3 are built**, both simplified from the
> text below by reader request — automatic, no per-word UI. Pass 1
> (`POST /api/vocab/anki-import`) imports only cards past Anki's new/learning
> queues (`-is:new -is:learn`) as `known`; it does not import queued/learning
> cards as `learning`, since only the review pile was asked for. Pass 3
> (`GET /api/vocab/frequency-summary` + `/frequency-queue` +
> `POST /api/vocab/frequency-commit`, read-stats' `#vocab` → frequency
> section) is a rank threshold with a preview, not a swipe: one click marks
> everything at or under the threshold `known`. Both resolve a source with no
> reading (Anki's field, a bare frequency term) against the master
> dictionary — zero candidates stores an empty reading, one is used directly,
> more than one (a homograph) is skipped and counted rather than guessed at,
> left for ordinary encounter-based triage to sort out once actually read.
>
> First real runs: Anki import landed **1,847 known, 73 skipped**; a
> frequency commit at rank ≤2000 landed **975 known, 159 skipped**. Ledger
> now stands at 2,731 known-in-master (up from 2,396 before either pass ran).
>
> | | |
> |---|---|
> | ✅ built | Passes 1, 2, 3 |
> | ❌ not built | the seed importer for epubs finished before tracking, Pass 4's periodic re-surfacing |

## Strategy: Multiple complementary passes

No single method captures everything. Combine several approaches, each catching
words the others miss.

### Pass 1: Anki Import

**What:** Export existing Anki deck(s) as CSV/JSON, extract target words, look up
lemma + reading via morphological analyzer, insert as `learning` or `known`.

**Why first:** Lowest effort, highest confidence. These are words you're actively
studying — you definitely know or are learning them.

**Expected yield:** ~1,995 words, of which **1,564 have no ledger row at all**
(measured 2026-07-26).

That last figure is the reason this pass survives as a pass. The Anki
*snapshot* already syncs into `vocabulary.mined`, but a sync only flags rows
that exist, and a mined word never met in the hooked line stream has none. The
1,564 misses are mostly multi-word expressions Sudachi never emits whole
(腹を探る, 相好を崩す, 意に介さず) and words mined from yt-mine or manga-mine
rather than read in a VN. No amount of re-tokenizing reaches them.

**Implementation notes:**
- The deck is already reachable over AnkiConnect (`services/anki.rs` →
  `anki_notes`), so no `.apkg` parsing is needed. The VocabKanji field holds a
  dictionary form.
- **Anki has no reading**, and the ledger's key needs one. Resolve it against
  the master dictionary: one candidate reading → take it; several (a homograph)
  → that note needs a human, and there are few enough to ask about; none →
  the term is not master vocabulary, so store it with an empty reading and let
  the dictionary flags say what it is.
- **Status: `known` for cards past Anki's new/learning queues, `learning` for
  those still inside them** (decided 2026-07-27, revising the line below). A
  card in active review is ~90% reliable evidence and the vocabulary count is an
  estimate regardless; a card still in the *new* queue is a word explicitly not
  yet had. The gate is `findNotes "deck:X -is:new -is:learn"` — note that
  `anki_notes` carries no card state today, so this needs a second query at
  import time or a wider snapshot.
  *(Superseded: "Status: `learning`, not `known`. Having a card is why the word
  is in the ledger, not evidence you have it yet — and `mined` already records
  the card." The queue distinction is what that line was missing.)*
- **Reader-triggered, not part of the recurring refresh.** The import is the
  reader asserting "trust my deck" once. Putting the same logic in the periodic
  Anki sync would break the rule that no sync writes `status`.
- Handle duplicates: if a row exists, set status but never touch its counts.

### Pass 2: Mass Read Calibration

**What:** Feed Japanese text you already understand well through the tokenizer.
Extract all unique (lemma, reading) pairs. Present them in bulk for rapid
confirmation — default to `known`, flag any you don't actually know.

**Why:** Captures the large passive vocabulary gap between your Anki cards and
actual reading ability. 5-10 chapters of familiar text could yield thousands of
words.

**Half of this pass is already done.** Everything hooked or pasted is in the
ledger with a real encounter count — 7,949 terms, and `POST /api/vocab/rebuild`
re-derives them from the whole history. "Or pasted" is load-bearing and now
true: `ingest::ingest_new_sessions` tokenizes `manual_sessions.content` into the
same ledger behind its own watermark, so an article or a typed-up paper book
already counts as calibration text without any new importer.

**(a) The triage UI is now built** — `#vocab` in read-stats. Sorting by
encounter count is what makes it cheap, exactly as this section argued: the
most-met words are the ones every downstream feature hits most.

What remains is (b) the **seed importer**: feeding in epubs of things finished
*before* tracking existed, which is the only way to reach text the line stream
never saw. It is narrower than it looks — pasting a chapter into a logged
session already routes through the same tokenizer and into the same ledger, so
the importer is a convenience over an existing path rather than a new pipeline,
and anything imported that way shows up in the triage queue automatically.

One correction to the paragraph above: the UI does **not** default everything to
`known`. Only words never looked up are ticked, whatever their encounter count —
see the status note at the top. That is the same concern as the **Risk** below,
handled by the data rather than by accepting false positives.

**Good calibration sources:**
- Light novels or VNs you've already finished
- News articles you've read
- Textbook passages at your level

**Implementation notes:**
- Tokenize all text, collect unique lemmas
- Subtract already-known words (from Anki import)
- Present remaining words grouped by frequency (most common first)
- UI should support rapid triage: default "known", one-click to mark "unknown"
- Consider showing the word in one of its original sentences for context
- **Built:** `read-stats/static/panels/triage.js`. yt-mine's `/vocab` page still
  writes its own superseded store on a `seen`/`known`/`blacklisted` vocabulary
  (`spec/knowledge-db.md` note 8) and is the one left to re-point.

**Risk:** You may recognize a word in context but not in isolation. Showing it
with a source sentence helps, but some false positives are acceptable — they'll
self-correct as you use the tool.

### Pass 3: Frequency List Triage

**What:** Load a Japanese word frequency list (e.g. Innocent Corpus top 10k,
or BCCWJ frequency data). Filter out words already in the DB. Present the rest
in frequency order for rapid known/unknown classification.

**Why:** Catches high-frequency words that didn't appear in your calibration
texts. Also quickly identifies your frequency-rank ceiling — the point where
most words become unknown.

**UI:** Rapid-fire, one word at a time. Show: lemma, reading, brief gloss.
Arrow keys or swipe: known / unknown / skip. Target speed: 50-100 words/minute.

**Implementation notes:**
- **The list is already loaded.** A BCCWJ frequency dictionary sits in
  `dictionary_frequency` with 886,343 rows — this pass needs no new data
  source, only a query. (That is the *word* frequency dictionary; the BCCWJ
  table compiled into `jp_core::text::bccwj_data` is kanji-only and unrelated.)
- Rank against BCCWJ but **filter to the master dictionary**, for the same
  reason the vocabulary denominator does: Jitendex-style phrase headwords would
  fill the queue with `ああでもないこうでもない` (`spec/knowledge-db.md`).
- Subtract rows already carrying a status; `new` is exactly the "not yet
  judged" filter, which is why it is a distinct value from `unknown`.
- Frequency entries carry no reading, so resolve one against the master
  dictionary the same way Pass 1 does.
- The point where the unknown rate climbs *is* the frequency-rank ceiling —
  worth recording, since it is the number that says how far down the list is
  worth triaging at all.

### Pass 4: Ongoing Passive Tracking

**What:** As you use the tool for reading/watching, track encounters. Words you
encounter repeatedly without looking up are candidates for `known`.

**Why:** Fills in the long tail of words that no calibration pass catches.

**Important caveats:**
- Do NOT auto-promote to `known`. Seeing a word 10 times doesn't mean you know
  it — you might be skipping it every time. The schema enforces this rather
  than trusting it: no sync or ingest path writes `status` at all, so
  auto-promotion cannot happen by accident.
- Instead: periodically surface "frequently seen, still **`new`**" words for
  manual review. "You've seen 散々 12 times this month and never looked it up.
  Do you know this word?" That query is `status = 'new' AND encounter_count >
  n AND lookup_count = 0` — and it only means anything because `new` was kept
  distinct from a judged `unknown`.
- The ledger already carries everything this pass reads: `encounter_count`,
  `lookup_count`, `first_seen`, `last_seen`, refreshed on every Anki refresh.

## Success Criteria

After passes 1-3, the vocabulary DB should:
- Contain 3000-8000+ known words (reasonable for someone reading books)
- Have few enough false positives that highlighting is useful (not everything
  marked unknown)
- Be good enough that i+1 filtering produces reasonable results

Perfection is not required. The system self-corrects through daily use.

## Priority

**This was the first thing to build**, and Pass 2's ledger half now is
(2026-07-27). It went first because it is the only pass needing no reading
resolution — the ledger already stores a real `(headword, reading)` from the
tokenizer, while passes 1 and 3 have to *infer* a reading from a bare headword
and hand homographs to a human.

**Next**, in the order that gets the most out of the least work:

1. **Run the triage pass.** It exists; the ledger is still all `new` until
   somebody sweeps it. This is reading, not coding.
2. **Pass 1 (Anki import).** ~1,564 deck words have no ledger row at all, so
   this *adds* vocabulary rather than only judging what reading produced. The
   queue-gated `known`/`learning` rule is settled (see Pass 1).
3. **Pass 3 (frequency list).** The only pass that reaches words never
   encountered, and the one this document says is load-bearing for approaching a
   real passive vocabulary. Its ambiguity UI is cheaper to build once Pass 1 has
   one.
4. **Pass 4** is then nearly free: its query is the same predicate the triage
   preselect already uses, so it is a re-run of an existing screen rather than
   new logic.

## The lexeme layer (built 2026-07-29)

Every pass above asserts things about `(headword, reading)` — an orthographic
*form*. Counting those forms is not counting words: 叔父, 伯父 and おじ are
three rows and one word, and a seeding pass that imports spellings in bulk
inflates the figure it exists to raise.

`jp_core::knowledge::lexeme` collapses forms to words at read time. Two rules
about it:

- **It is derived, never stored.** There is no `redundant` column and there
  must not be one: a flag written at import time depends on what was already
  in the ledger, so importing Anki before jiten.moe and jiten.moe before Anki
  would leave different databases. Derived, every order converges — which is
  what makes a bulk seed safe to run repeatedly and in any sequence.
- **Counting and asking are different questions.** `known_lexemes` collapses
  in both directions (叔父 ≡ 伯父 ≡ おじ, one word). `redundant_forms` — what
  triage should stop offering — runs one direction only: a form is settled by
  a known form whose kanji are a superset of its own. Knowing 零れ落ちる
  settles こぼれ落ちる; knowing こぼれ落ちる settles nothing about whether 零
  can be read. Running that both ways would silently mark unread kanji
  spellings known.

The grouping comes from JMdict `ent_seq`, carried in Yomitan term-bank field 6
and now stored as `dictionary_entries.sequence`. Jitendex supplies it (293k
entries); Sankoku publishes ids too but splits 叔父 from 伯父, so the
larger-coverage dictionary wins. The two roles stay independent and both are
needed: **the master dictionary decides what counts as vocabulary, a reference
dictionary decides which rows are the same word.**

First measurement: 6,207 known in-master forms → **6,098 words**. The 109
collapsed are pure spelling — アイディア/アイデア, 飲む/呑む, 身体/体/躯,
奴/ヤツ.

## Pass 5: the jiten.moe export (built 2026-07-29)

The first source that names **words** rather than spellings. jiten.moe's JSON
export is a list of JMdict entry ids (`w` = `ent_seq`), which is the same key
`dictionary_entries.sequence` now stores.

That removes the problem passes 1 and 3 were built around. Both had to infer a
reading from a bare headword and skip whatever came back ambiguous — 73 and 159
terms. This pass guesses nothing: 辛い/つらい (1365860) is in the export and
辛い/からい (1365850) is not, so exactly one of them is marked. There is no
ambiguous-skipped count because there is no ambiguity.

An id fans out to every spelling of it the master dictionary lists, which is
only safe because counting collapses back: こちら and こっち are marked
separately and reported as one word. The fan-out buys triage not asking about
each spelling in turn; the lexeme layer stops it inflating the total.

Two rules the import obeys, both learned the hard way in the first dry run:

- **It seeds, it does not overrule.** `seed_status_each` writes only where the
  status is still `new`. The first run used `set_status_each` and turned 16
  rows the reader had judged `unknown` into `known` — a bulk assertion made
  months ago elsewhere overruling one made here with the word on screen.
- **It refreshes the dictionary flags afterwards.** Most of what it writes are
  rows that did not exist: words never met in any reading, whose flags are all
  zero. Without the refresh, `in_master` excluded the entire import from the
  vocabulary scale — 6,255 seeded words counted as nothing.

First real run, against the frozen dev copy of the live data:

| | |
|---|---|
| cards / distinct ids | 14,132 / 13,312 |
| entries with a master spelling | 10,544 |
| dropped (names, JMdict-only phrases) | 2,768 |
| spellings marked known | 7,272 |
| **known words** | **6,098 → 11,694** |

Note the spread the lexeme layer now covers: 13,479 known in-master spellings
report as 11,694 words.
