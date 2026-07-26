# Cold Start — Bootstrapping the Knowledge Base

This is the most critical problem to solve. Every downstream feature (highlighting,
i+1 filtering, card mining) depends on an accurate vocabulary database. The goal
is to go from zero to a reasonable approximation of your actual knowledge quickly.

> **Status (2026-07-26).** The ledger these passes fill now exists and holds
> **7,949 terms** backfilled from the whole reading history
> (`spec/knowledge-db.md`, migration note 4). Every one of them is `status =
> 'new'`: nothing has been asserted yet, because no pass below is built. The
> plumbing is done; the passes are the work.
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
- Status: `learning`, not `known`. Having a card is why the word is in the
  ledger, not evidence you have it yet — and `mined` already records the card.
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
re-derives them from the whole history. What remains is (a) the triage UI over
those rows, and (b) the **seed importer**: feeding in epubs of things finished
*before* tracking existed, which is the only way to reach text the line stream
never saw. Sorting by encounter count is what makes the triage cheap — the
~1,000 terms seen ten or more times are the ones worth a decision, and the tail
below that can wait for the frequency pass to reach it.

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

**This is the first thing to build.** Without it, every other feature is noise.
As of 2026-07-26 the ledger underneath it is built and full, and every row says
`new` — so the passes above are now the *only* thing standing between the data
and the highlighting, i+1 filtering and unknown-word counts that depend on it.
