# Kotodex — Pokedex for Japanese Words

> **Rough brainstorm — nothing decided.** This is an unfleshed-out idea dump,
> not a design or a commitment. It may never be built. Kept only so the thoughts
> aren't lost.

**Concept:** Your journey through Japanese is like filling out a Pokedex. Every word
you encounter is a creature to discover, study, and master. The Kotodex gives you a
visual, motivating collection view of your vocabulary — where you found each word,
how well you know it, and how your collection grows over time.

## Name candidates

| Name | Origin | Feel |
|------|--------|------|
| **Kotodex** | 言 (koto, word/language) + dex | Clean, direct, memorable |
| **Kotoba Zukan** | 言葉図鑑 (word encyclopedia) | Japanese "illustrated guide" — what the Pokedex literally is |
| **Gogodex** | 語 (go, language/word) + dex | Shorter, punchier |
| **Tangodex** | 単語 (tango, vocabulary word) + dex | Most literal: "vocab-dex" |
| **Mojidex** | 文字 (moji, character) + dex | Better if kanji-focused |
| **Gengokan** | 言語館 (language hall/museum) | Feels like a place to explore |
| **Kotomon** | 言 + mon (monster) | Leans into the Pokemon parallel harder |

Working title: **Kotodex** (best balance of clarity and personality).

## The core metaphor

| Pokemon | Kotodex |
|---------|---------|
| Pokemon species | A word (lemma + reading) |
| Pokedex entry | Word detail page: meaning, encounters, mastery |
| Seen vs caught | Encountered vs known |
| Pokedex number | Frequency rank (most common = #001) |
| Pokemon types | Part of speech: verb, noun, adjective, adverb, particle... |
| Regions | JLPT level (N5 = Kanto, N1 = the final frontier) |
| Evolution chain | Word families: 食 → 食べる → 食べ物 → 食事 → 食堂 |
| Habitat | Media source: YouTube, VN, book, web |
| Rarity | Frequency band: common / uncommon / rare / legendary |
| Shiny | Words you encounter in surprising/unusual readings |
| Trainer level | Overall stats: total known, % coverage at each level |

## What the Pokedex metaphor gives you

1. **Completion drive.** Seeing "347 / 800 N3 words discovered" taps into the same
   compulsion as filling a Pokedex. You want to close the gaps.

2. **Every word has a story.** The Pokedex entry for Charizard tells you where it
   lives, what it eats. Your entry for 取り消す tells you: "First encountered in
   Steins;Gate, Chapter 3. Seen 7 times across 3 sources. You looked it up twice."

3. **Progress is always visible.** Even on days when study feels slow, you can see
   your collection growing. New encounters light up. Status upgrades feel like
   evolutions.

4. **Natural categorization.** The Pokedex is organized by region and type. The
   Kotodex is organized by JLPT level, frequency band, part of speech, and
   media source — all of which are natural axes for vocabulary.

## Feature ideas

### The Collection Grid

The main view. A grid of word "tiles" organized by category (JLPT level, frequency
band, or semantic group).

- **Known words** — full color, showing the kanji/word
- **Seen/learning words** — visible but slightly muted, maybe with a progress ring
- **Undiscovered words** — silhouettes or greyed-out placeholders (you can see *how many*
  words remain in a category without seeing what they are)

Think the Pokedex grid: rows of small squares, each one either filled in or a mystery.
Clicking one opens the detail view.

**Variants:**
- Dense grid (hundreds visible, just colored dots — heatmap style)
- Card grid (bigger tiles, showing word + reading + status)
- List view (sortable table for power users)

### Word Detail Page (the "Pokedex entry")

When you tap a word, you get a rich entry page:

- **Header:** kanji, reading, pitch accent, POS badge (like a Pokemon type badge)
- **Status ring:** visual indicator of mastery (unknown → seen → learning → known),
  like a CP ring or experience bar
- **Stats block:**
  - Times encountered (total)
  - Times looked up
  - First seen: date + source ("Steins;Gate, Episode 4")
  - Last seen: date + source
  - SRS status (if Anki-linked): interval, next review
  - Streak: consecutive days encountered (or reviewed)
- **Encounter log:** scrollable list of every context sentence where you met this word,
  with source and timestamp — like a Pokemon's "caught at" location history
- **Word family / evolution chain:** related words sharing the same kanji or root,
  shown as a visual tree or chain
- **LLM explanation:** cached contextual explanation (expandable)
- **Dictionary entries:** from loaded Yomitan dictionaries

### Progress Dashboard ("Trainer Card")

Your overall stats page, like the trainer card in Pokemon:

- **Total discovered / Total in database** (with a satisfying progress bar)
- **Breakdown by JLPT level** — five progress bars, one per level
  - "N5: 623/800 (78%)" with a colored fill bar
- **Breakdown by frequency band** — top 1k, 2k, 5k, 10k, 20k+
- **Words over time** — line chart showing cumulative known words by date
- **Discovery rate** — "This week: 34 new words encountered, 12 leveled up"
- **Collection milestones / badges:**
  - "First 100 words" (starter badge)
  - "N5 Complete" (region badge)
  - "1000 words known" (trainer rank up)
  - "Encountered in 5+ sources" (well-traveled word)
  - "100-day streak" (dedication badge)
- **Rarest catch:** your lowest-frequency known word (bragging rights)
- **Type chart:** radar chart or bar chart of POS distribution — are you
  verb-heavy? noun-heavy? neglecting adverbs?

### Encounter Map

A visual representation of *where* your words come from:

- Each media source (a YouTube channel, a VN, a book) is a "region" or "route"
- Regions sized by how many words you discovered there
- Could be: a treemap, a bubble chart, or an actual stylized map
- Clicking a region shows all words first encountered in that source

### Evolution Chains

Words that share kanji form natural "evolution chains":

```
食 (た.べる / く.う / ショク)
├── 食べる (to eat) — base form
├── 食べ物 (food)
├── 食事 (meal)
├── 食堂 (cafeteria)
├── 食料 (provisions)
└── 食欲 (appetite)
```

Visualized like Pokemon evolution: a connected graph where discovering related
words fills out the chain. Having 4/6 in a family is motivating — you want to
complete the set.

### Daily Summary ("Professor Oak's Report")

A short daily or session-end summary:

- "Today you encountered 142 words across 3 videos. 8 were new discoveries!"
- "取り返す evolved to Known status. Congratulations!"
- "You're 3 words away from completing the N4 Verbs collection."
- "Your rarest find today: 齟齬 (frequency rank #18,420). A legendary catch!"

Tone: encouraging, specific, never patronizing. Like the Pokedex evaluation in
Pokemon games after each gym.

### Achievements / Badges

Specific, earnable milestones:

**Collection badges:**
- Starter (first 10 words known)
- Collector (100, 500, 1000, 2000, 5000, 10000)
- N5/N4/N3/N2/N1 Master (complete a JLPT level)
- Polytype (know 50+ words in every POS category)

**Encounter badges:**
- Well-Read (encountered words from 10+ different sources)
- Deep Diver (100+ encounters in a single source)
- Daily Discovery (encounter a new word 30 days in a row)
- Completionist (fill out an entire evolution chain)

**Study badges:**
- Quick Learner (word goes from unknown → known in under a week)
- Long Game (a word you encountered 50+ times finally levels up)

## Scope: words, kanji, or both?

**Start with words.** The existing data model is word-centric (lemma + reading).
Kanji tracking is a different axis — one kanji appears in many words, and kanji
knowledge (readings, stroke order, radicals) is a deeper rabbit hole. The spec
already notes "Kanji knowledge — Deferred."

But the evolution chain feature naturally bridges into kanji awareness: grouping
words by shared kanji shows kanji as the *connective tissue* between words, even
without a full kanji tracking system.

**Future:** a Kanji sub-dex could show each kanji as its own entry, with all the
words containing it as its "moves" or "abilities." But that's a later expansion.

## Integration: new tool or part of yt-mine?

Three options:

### Option A: Feature within yt-mine (extend the existing frontend)

Add Kotodex as a new route/section in the yt-mine Preact SPA. The vocabulary data
is already in SQLite. Just add new API endpoints and frontend pages.

**Pro:** No new infrastructure. Vocabulary and encounters already flow through yt-mine.
**Con:** yt-mine becomes "jp-tools-frontend" — the name stops making sense. The
responsibility creep makes the codebase harder to reason about.

### Option B: Separate frontend, shared database

A new crate (`kotodex/`) with its own Axum server and frontend, reading from the
same SQLite database as yt-mine. jp-core stays the shared library.

**Pro:** Clean separation. Each tool has one job. Can run independently.
**Con:** Two servers, potential SQLite write contention (mitigated by WAL mode),
some shared types/queries.

### Option C: Unified frontend shell, feature crates

One web server (`jp-web/` or similar) that hosts all frontend features — sentence
mining, kotodex, future texthooker — as routes in a single SPA. yt-mine's current
frontend migrates into this. Backend logic stays in feature crates.

**Pro:** Single entry point for the user. Shared navigation, consistent UI.
**Con:** Bigger refactor. Need to extract yt-mine's mining logic from its web layer.

### Recommendation

**Start with Option A** (path of least resistance), but keep the Kotodex routes and
components cleanly separated in their own feature folder within the frontend. If/when
it outgrows yt-mine, extract into Option C. The existing frontend already uses
feature-folder organization (per recent commits), so this fits naturally.

The key insight: Kotodex is primarily a *read-only view* over data that already
exists (vocabulary + encounters). It doesn't need its own write pipeline. It just
needs new API endpoints to query and aggregate, plus a rich frontend.

## New API endpoints (sketch)

```
GET  /api/kotodex/overview       — total counts, JLPT breakdown, recent activity
GET  /api/kotodex/words          — paginated grid data with filters (status, JLPT, POS, source)
GET  /api/kotodex/words/:id      — full detail for one word (encounters, family, explanation)
GET  /api/kotodex/words/:id/family — evolution chain (words sharing kanji)
GET  /api/kotodex/sources        — list of media sources with word counts
GET  /api/kotodex/achievements   — earned and in-progress badges
GET  /api/kotodex/daily-summary  — today's stats
```

## Data model additions

The existing `vocabulary` and `encounters` tables cover most needs. Additions:

```sql
-- JLPT level for each word (populated from a frequency/JLPT list)
ALTER TABLE vocabulary ADD COLUMN jlpt_level INTEGER;  -- 5, 4, 3, 2, 1, or NULL

-- Frequency rank (from a standard frequency list)
ALTER TABLE vocabulary ADD COLUMN frequency_rank INTEGER;

-- Achievement tracking
CREATE TABLE achievements (
    id TEXT PRIMARY KEY,           -- 'collector_100', 'n5_master', etc.
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    earned_at TEXT,                -- NULL if not yet earned
    progress INTEGER DEFAULT 0,   -- for progressive achievements
    target INTEGER                -- goal value
);
```

JLPT level and frequency rank could also come from an external list joined at query
time rather than stored on the vocabulary row. Worth deciding during implementation.

## Visual style notes

The Pokedex has a distinctive look: red casing, pixel-art sprites, terse data
entries. The Kotodex doesn't need to literally look like a Pokedex, but it should
borrow the *feeling*:

- **Dense but scannable.** Lots of small entries visible at once.
- **Satisfying state transitions.** When a word levels up, it should feel like
  something happened — color change, subtle animation, a small celebration.
- **Stats-forward.** Numbers, progress bars, completion percentages front and center.
- **Personality.** The daily summary, achievement names, and rarity labels inject
  character into what could otherwise be a dry vocabulary spreadsheet.

Color palette could follow the knowledge status:
- Unknown/undiscovered: grey silhouette
- Seen: blue (like a "seen" Pokedex entry)
- Learning: amber/orange (in progress, warming up)
- Known: green (fully caught, mastered)
