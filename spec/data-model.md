# Data Model

SQLite, single-user, local-first. No need for Postgres — this is a personal
tool with a single writer.

## Schema

### vocabulary

The core knowledge state. One row per unique (lemma, reading) pair.

```sql
CREATE TABLE vocabulary (
    id INTEGER PRIMARY KEY,
    lemma TEXT NOT NULL,              -- base form: 食べる
    reading TEXT NOT NULL,            -- kana: たべる
    pos TEXT,                         -- part of speech: 動詞-自立
    status TEXT NOT NULL DEFAULT 'unknown',
    encounter_count INTEGER NOT NULL DEFAULT 0,
    first_seen TEXT,                  -- ISO 8601 timestamp
    last_seen TEXT,                   -- ISO 8601 timestamp
    source TEXT,                      -- how it entered the DB (see below)
    UNIQUE(lemma, reading)
);

CREATE INDEX idx_vocab_status ON vocabulary(status);
CREATE INDEX idx_vocab_lemma ON vocabulary(lemma);
```

**status values:**
- `unknown` — default, never encountered or explicitly marked unknown
- `seen` — encountered in media but not yet assessed
- `learning` — actively studying (e.g. has an Anki card)
- `known` — confident recognition in context

**source values:** `calibration`, `anki_import`, `frequency_list`, `manual`,
`mined`, `yomitan`

### encounters

Every occurrence of a word in consumed media. Enables "show me every context
where I saw this word" queries. Also feeds encounter_count on vocabulary.

```sql
CREATE TABLE encounters (
    id INTEGER PRIMARY KEY,
    vocab_id INTEGER NOT NULL REFERENCES vocabulary(id),
    sentence TEXT NOT NULL,
    source_type TEXT NOT NULL,        -- 'vn', 'youtube', 'book', 'web'
    source_title TEXT,                -- 'Steins;Gate', 'N1 Reading Practice', etc.
    created_at TEXT NOT NULL          -- ISO 8601
);

CREATE INDEX idx_encounters_vocab ON encounters(vocab_id);
CREATE INDEX idx_encounters_source ON encounters(source_type, source_title);
```

### word_explanations

Cached LLM-generated explanations. Expensive to produce, so we store them.
The same word in different contexts may warrant different explanations.

```sql
CREATE TABLE word_explanations (
    id INTEGER PRIMARY KEY,
    vocab_id INTEGER NOT NULL REFERENCES vocabulary(id),
    sentence_context TEXT,            -- the sentence that prompted the lookup
    explanation TEXT NOT NULL,         -- the LLM response (markdown)
    model TEXT,                       -- model identifier for provenance
    created_at TEXT NOT NULL
);

CREATE INDEX idx_explanations_vocab ON word_explanations(vocab_id);
```

### cards

Staged flashcard candidates. Reviewed and cherry-picked before export to Anki.

```sql
CREATE TABLE cards (
    id INTEGER PRIMARY KEY,
    vocab_id INTEGER NOT NULL REFERENCES vocabulary(id),
    sentence TEXT NOT NULL,
    translation TEXT,                 -- optional, LLM-generated
    audio_path TEXT,                  -- relative path to audio clip
    image_path TEXT,                  -- relative path to screenshot
    source_type TEXT NOT NULL,
    source_title TEXT,
    status TEXT NOT NULL DEFAULT 'pending',  -- pending | accepted | rejected | exported
    created_at TEXT NOT NULL
);

CREATE INDEX idx_cards_status ON cards(status);
CREATE INDEX idx_cards_vocab ON cards(vocab_id);
```

## Design Decisions

### Why (lemma, reading) as the unique key?

Japanese has extensive homography. 今日 can be きょう (today) or こんにち (as in
こんにちは). 上 has over a dozen readings. "Knowing" a word means knowing it in
a specific reading. The morphological analyzer provides both lemma and reading,
so this is a natural key.

### Why separate encounters from vocabulary?

Vocabulary tracks your knowledge state — a slowly-changing dimension. Encounters
are an append-only log of raw data. Keeping them separate means:
- Vocabulary table stays small and fast for highlighting lookups
- Encounter history is available for "where did I see this?" queries
- encounter_count on vocabulary is a denormalized counter for convenience

### Why stage cards instead of sending directly to Anki?

Bulk mining (e.g. processing an entire VN chapter or anime episode) produces
many candidates. Most won't be worth studying. The staging table lets you review,
filter (i+1, quality), and pick before exporting a clean batch to Anki.

### What's NOT in the model

- **Grammar patterns** — Deferred. Tracking grammar (e.g. ～させられる as
  causative-passive) is a different problem from vocabulary. May add later as a
  separate table.
- **Kanji knowledge** — Deferred. Kanji decomposition and knowledge tracking is
  a rabbit hole. Tools like kanji.garden handle this well already.
- **SRS scheduling** — Anki handles this. No need to reimplement.
