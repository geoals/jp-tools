# Cold Start — Bootstrapping the Knowledge Base

This is the most critical problem to solve. Every downstream feature (highlighting,
i+1 filtering, card mining) depends on an accurate vocabulary database. The goal
is to go from zero to a reasonable approximation of your actual knowledge quickly.

## Strategy: Multiple complementary passes

No single method captures everything. Combine several approaches, each catching
words the others miss.

### Pass 1: Anki Import

**What:** Export existing Anki deck(s) as CSV/JSON, extract target words, look up
lemma + reading via morphological analyzer, insert as `learning` or `known`.

**Why first:** Lowest effort, highest confidence. These are words you're actively
studying — you definitely know or are learning them.

**Expected yield:** ~1500 words (your current deck size).

**Implementation notes:**
- Anki export formats: `.apkg` (SQLite inside a zip), or export as
  tab-separated text from within Anki
- Need to identify which field contains the target word — varies by note type
- Some cards may have sentences rather than single words; run through tokenizer
  to extract the target
- Handle duplicates: if a word already exists, update status but don't overwrite

### Pass 2: Mass Read Calibration

**What:** Feed Japanese text you already understand well through the tokenizer.
Extract all unique (lemma, reading) pairs. Present them in bulk for rapid
confirmation — default to `known`, flag any you don't actually know.

**Why:** Captures the large passive vocabulary gap between your Anki cards and
actual reading ability. 5-10 chapters of familiar text could yield thousands of
words.

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
- Source: Innocent Corpus word frequency list is well-suited for media consumers
  (derived from novels). BCCWJ is more balanced across genres.
- Need a mapping from frequency list entries to (lemma, reading) pairs
- Some frequency lists include readings, some don't — may need to generate
  readings via the morphological analyzer

### Pass 4: Ongoing Passive Tracking

**What:** As you use the tool for reading/watching, track encounters. Words you
encounter repeatedly without looking up are candidates for `known`.

**Why:** Fills in the long tail of words that no calibration pass catches.

**Important caveats:**
- Do NOT auto-promote to `known`. Seeing a word 10 times doesn't mean you know
  it — you might be skipping it every time.
- Instead: periodically surface "frequently seen, still marked unknown" words for
  manual review. "You've seen 散々 12 times this month and never looked it up.
  Do you know this word?"

## Success Criteria

After passes 1-3, the vocabulary DB should:
- Contain 3000-8000+ known words (reasonable for someone reading books)
- Have few enough false positives that highlighting is useful (not everything
  marked unknown)
- Be good enough that i+1 filtering produces reasonable results

Perfection is not required. The system self-corrects through daily use.

## Priority

**This is the first thing to build.** Without it, every other feature is noise.
