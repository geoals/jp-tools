# Architecture

## High-Level Components

```
┌─────────────────────────────────────────────────────┐
│                   Browser                            │
│  ┌──────────────┐  ┌────────────┐  ┌─────────────┐ │
│  │ Texthooker   │  │  Triage UI │  │ Card Review │ │
│  │ Page         │  │ (freq list │  │ UI          │ │
│  │ + highlighting│  │  calibrate)│  │             │ │
│  └──────┬───────┘  └─────┬──────┘  └──────┬──────┘ │
│         │                │                 │        │
│         │       Yomitan works here         │        │
│         │       (independent, no coupling) │        │
└─────────┼────────────────┼─────────────────┼────────┘
          │                │                 │
          ▼                ▼                 ▼
┌─────────────────────────────────────────────────────┐
│                  Local HTTP API                      │
│                                                      │
│  POST /tokenize        — text → tokens               │
│  GET  /vocab/:lemma    — lookup word status           │
│  POST /vocab/status    — update word status           │
│  POST /annotate        — text → HTML with highlights  │
│  POST /explain         — word + context → LLM call    │
│  POST /cards           — create card candidate        │
│  GET  /cards?status=   — list cards for review        │
│  POST /cards/:id/export — export to Anki             │
│  POST /calibrate       — bulk text → known words      │
│  ...                                                  │
└──────────┬──────────────┬────────────────────────────┘
           │              │
     ┌─────▼─────┐  ┌────▼──────┐
     │ Tokenizer │  │  SQLite   │
     │ (LinDera/ │  │  DB       │
     │  MeCab)   │  │           │
     └───────────┘  └───────────┘
           │
     ┌─────▼──────────┐
     │ LLM API        │
     │ (Claude/etc)   │
     │ for word       │
     │ explanations   │
     └────────────────┘
```

## Technology Choices

### Backend: Rust

**Why Rust:**
- LinDera and Vibrato are Rust-native — no FFI needed for morphological analysis
- Single binary deployment, no runtime dependencies
- SQLite via `rusqlite` is mature
- Axum or Actix-web for the HTTP API
- Aligns with your existing Rust experience and conventions

**Alternative considered:** Node/TypeScript or Python. Either would work, but
Rust avoids the "call MeCab via subprocess" pattern and keeps everything in one
process.

### Frontend: Lightweight Web UI

The texthooker page and triage UIs are simple HTML/JS/CSS apps served by the
backend. No framework needed initially — vanilla JS or a minimal framework
(Preact, htmx) keeps it simple.

**Why web:**
- Texthooker pages are already web-based (this is the existing convention)
- Yomitan is a browser extension — it needs a browser page to attach to
- Cross-platform for free

### Morphological Analyzer

**Recommended: LinDera with UniDic**

| Criterion         | LinDera                  | Vibrato               | MeCab (via FFI)       |
|-------------------|--------------------------|------------------------|-----------------------|
| Language          | Rust                     | Rust                   | C++ (need FFI)        |
| Bundled dicts     | IPAdic, UniDic, ko-dic   | None (load your own)   | N/A                   |
| Ease of setup     | `cargo add lindera`      | Need to compile dict   | System install + FFI  |
| Speed             | Fast                     | Very fast              | Very fast             |
| Ecosystem         | Used in Tantivy          | Newer, smaller         | Largest               |

LinDera is the pragmatic choice: easiest to get running in Rust, good enough
performance, ships with dictionaries. If performance becomes an issue (unlikely
for single-sentence tokenization), Vibrato is a drop-in replacement using the
same dictionary format.

**Dictionary: UniDic over IPAdic.** UniDic provides better lemmatization
(critical for DB lookups) and includes accent information. IPAdic's lemma field
is less consistent.

### LLM Integration

HTTP calls to an external LLM API (Claude, OpenAI, etc.). No local model needed
— the quality difference matters for nuanced word explanations.

**Pattern:**
1. Morphological analyzer provides: surface, lemma, reading, POS
2. These are injected into a structured prompt along with the sentence context
3. Response is cached in `word_explanations` table
4. Cache key: `(vocab_id, sentence_context)` — same word in same sentence
   returns cached result; same word in different sentence may get a fresh
   explanation

### Anki Integration

For card export, use AnkiConnect's HTTP API (localhost:8765). This is the
standard way to programmatically add cards to Anki.

For Anki import (cold start), parse the `.apkg` file directly — it's a zip
containing a SQLite database.

## What This Architecture Does NOT Include

- **No WASM tokenizer in the browser.** Simpler to tokenize server-side and
  return annotated HTML. Revisit if latency is a problem.
- **No background daemon.** The API server runs when you're studying. No need
  for it to be always-on.
- **No mobile support.** Desktop-first. VN reading and YouTube watching are
  desktop activities.
