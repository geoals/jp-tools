# yt-mine — YouTube Sentence Mining

Rust 2024 edition. Axum JSON API + Preact frontend (no build step), SQLite persistence.

## Pipeline

YouTube URL → yt-dlp download → whisper-service transcription → jp-core tokenization → sentence display → click a word → popup → pick the card's word → Anki export

Jobs run as background `tokio::spawn` tasks. Frontend polls via JSON API.

## Key design decisions

- **The tokenizer is `jp_core::highlight`'s, all seven inputs** — the same
  pipeline read-stats' reader and ingest use, not a bare Sudachi with the
  headword list. A transcript and a hooked VN line have to segment the same
  way, or a word mined here lands on a ledger key `#read` never produces. It
  also means `analyze` gives each token its ledger status for free
- **Traits for external tools** — `AudioDownloader`, `Transcriber`, `AnkiExporter`, `MediaExtractor`, `Tokenizer` (in jp-core), `LlmDefiner` enable mocking via `mockall`
- **Subprocesses over FFI** — clean boundary for yt-dlp, ffmpeg
- **Remote whisper-service** — transcription offloaded to separate FastAPI container (NDJSON streaming)
- **Preact + htm + signals from CDN** — no build step, ES module imports from esm.sh with pinned versions
- **JSON API + SPA shell** — `/api/*` returns JSON, `/` and `/{video_id}` serve the SPA shell

## The popup

A word in a sentence is clicked, not hovered, and what opens is the VN
overlay's popup — `static/features/mining/word-popup.js` against `/api/define`
and `/api/expand`, both thin wrappers on `jp_core::define`. So a word looked up
here and a word looked up over the game show the same dictionaries in the same
order, page the same way, and offer the same escape hatch when the tokenizer
was wrong about a position (経年劣化 split in two, 素振り read the other way).

Three differences from the overlay's, each for a reason:

- **Nothing records a lookup.** A lookup is a reading-session event and there
  is no session here, so `define` leaves `lookup_id` null.
- **＋ picks the card's word, it does not mine.** A video is mined a sentence
  at a time and the export button commits the batch; the overlay's ＋ adds a
  card immediately because there is no batch. The picked word rides in
  `selectedWords` with its reading, because a word picked out of the scan need
  not be a token at all and the server cannot look its reading back up.
- **✓ and ✗ are the only judging.** No side mouse buttons: the sentence list is
  an ordinary page, not a layer over a game.

Tokens carry their ledger status, and the three tints are read-stats' own —
`known` is deliberately not one of them, because the absence of a mark is what
makes the marks readable.

**Rows are rebuilt every render.** `SentenceList` used to cache a VNode per
sentence and hand back the same reference, which makes Preact skip the subtree
— including when a signal the row reads has changed. A word judged in the popup
kept its old tint and the open token stayed outlined after the popup closed.

## Not built

Smart filtering — frequency thresholds, i+1 sentence selection, dimming
sentences whose words are all known. The ledger is read now, so the data is
there; nothing consumes it beyond the tints.

## Tokenization & Dictionary

Provided by `jp-core` crate. See `jp-core/` for details.

- **Lookup goes through `jp_core::define`**, the same call the VN overlay's
  popup draws from, via `jp_mine_core::lookup::lookup_word` — which flattens it
  to the four things a card holds. So a card gets the dictionaries in reading
  order (明鏡, then the master) rather than install order, and its `Frequency`
  is `READER_FREQUENCY` — how common the word is in fiction. It used to be
  whichever frequency dictionary was installed first, which was BCCWJ:
  newspaper prose, where 素振り ranks 14,117 against 6,157 in fiction.

## Build & run

```sh
cargo run -p yt-mine                              # server on 0.0.0.0:3000
JP_TOOLS_FAKE_API=true cargo run -p yt-mine       # dev mode (no external deps)
cargo test -p yt-mine                             # unit + integration (mocked)
cargo test -p yt-mine -- --ignored                # real subprocess tests
```

## Config

Via env vars, loaded from `.env` (repo root) via `dotenvy`. See `config.rs` and
`.env.example`.

Anki export fields are all configurable via `JP_TOOLS_ANKI_*` vars (model, deck,
field mapping). Defaults match the "Japanese sentences" Yomitan note type — and
are now the same fields read-stats writes: the glossary goes to `VocabDefFull`
in Yomitan's per-dictionary markup (`VocabDef` is Yomitan's own short gloss and
not ours to overwrite), the pitch to `VocabPitchNum` + `VocabPitchPattern` as
markup rather than a bare number, and the LLM gloss to `CompactDef`.

**The card is built by `jp_mine_core::card`, the gloss by
`jp_mine_core::compactdef`.** yt-mine's own `LlmDefiner` prompt is gone: it was
a third paraphrase of the tag rubric, still on a four-tier familiarity scale
with no FLAVOR axis, which is exactly what `jp_mine_core::tags` exists to
prevent. `services::llm` is now just the trait and a thin impl, kept so the
fake and the route tests can stand in for the call.
