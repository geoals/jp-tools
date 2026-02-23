# jp-tools — YouTube Sentence Mining

Rust (edition 2024) tool for mining Japanese sentences from YouTube videos. Axum web server with htmx frontend, SQLite persistence.

## Architecture

Pipeline: **YouTube URL → yt-dlp download → faster-whisper transcription → Lindera tokenization → sentence display → target word selection → dictionary lookup → Anki export**

Jobs run as background `tokio::spawn` tasks. The frontend polls for status via htmx fragments.

### Project structure

```
src/
  lib.rs              — pub mod declarations (shared library crate)
  main.rs             — server bootstrap, wires concrete implementations
  app.rs              — AppState (DI container) + Axum router
  config.rs           — env-based configuration
  db.rs               — SQLite via sqlx, compile-time checked queries
  error.rs            — AppError enum → HTTP status mapping
  models.rs           — Job, JobStatus (state machine), Sentence, TranscriptSegment
  routes/
    mining.rs         — HTTP handlers, htmx polling, form handling, export orchestration
  services/
    pipeline.rs       — orchestrates download → transcribe → store
    download.rs       — AudioDownloader trait, YtDlpDownloader (subprocess)
    transcribe.rs     — Transcriber trait, WhisperWorker (persistent Python subprocess)
    export.rs         — AnkiExporter trait, AnkiConnectExporter (HTTP to localhost:8765)
    media.rs          — MediaExtractor trait, FfmpegMediaExtractor (screenshots + audio clips)
    tokenize.rs       — Tokenizer trait, LinderaTokenizer (UniDic, morphological analysis)
    dictionary/
      mod.rs          — Dictionary struct, loading, parsing, title/slug, wrap_definitions
      html.rs         — structured_content_to_html, html_escape, camel_to_kebab, render_style
      tests.rs        — all dictionary + HTML conversion tests
  bin/
    tokenize.rs       — CLI tool for testing tokenization output
scripts/
  transcribe.py       — faster-whisper wrapper, persistent worker mode (stdin/stdout JSON)
templates/
  base.html           — shared layout + CSS
  mining/             — htmx templates (submit, job status, export success)
```

### Design decisions

- **Traits for external tools** — `AudioDownloader`, `Transcriber`, `AnkiExporter`, `MediaExtractor`, `Tokenizer` are traits so tests can mock them via `mockall`
- **Subprocesses over FFI** — yt-dlp, faster-whisper, ffmpeg are external tools; subprocesses keep the boundary clean
- **Persistent whisper worker** — model loading is expensive; `WhisperWorker` spawns once and reuses via stdin/stdout JSON protocol
- **htmx over SPA** — server-rendered HTML fragments, no JS framework
- **lib.rs + main.rs split** — shared library crate so `src/bin/` tools reuse the same modules

### Tokenization

Lindera with embedded UniDic dictionary. Produces morpheme-level tokens (短単位) which are finer-grained than dictionary entries. Known limitation: verb conjugations are split (e.g. み+られ+ます instead of みられます), and non-content-word tokens (particles, auxiliaries) are not clickable in the UI.

Current `is_content_word` POS filter: 名詞, 動詞, 形容詞, 形状詞, 副詞.

### Dictionary lookup

Loads one or more Yomitan-format zip files. Lookup is exact match on headword (HashMap). The target word sent from the UI is the token's `base_form` (dictionary form from UniDic). If no match in any dictionary, VocabDef is left empty on the Anki card.

Multiple dictionaries are supported — results from each are concatenated in the VocabDef field, each wrapped with title/body divs for per-dictionary CSS styling.

Pitch accent data is read from `term_meta_bank_*.json` files inside Yomitan zips. Entries where the second element is `"pitch"` are parsed; `"freq"` and other meta types are skipped. On export, VocabFurigana uses Anki bracket notation (`食べる[たべる]`) and VocabPitchNum is the downstep position (e.g. `0`, `2`, or `0,3` for multiple accents). Both fields use the first dictionary that provides the data.

### Structured-content HTML conversion

Yomitan structured-content JSON (lists, ruby text, example sentences, links) is converted to semantic HTML by `structured_content_to_html()` in `dictionary/html.rs`. The conversion is a recursive descent that handles strings, arrays, tag objects with attributes (lang, title, href, data-*, style), void elements (br), and skips images. Styling is separated from markup — the HTML uses `data-content`/`data-class` attributes that can be targeted by CSS in the Anki card template.

Each dictionary's definitions are wrapped with `<div class="dict-{slug}-title">` and `<div class="dict-{slug}-body">` where the slug is derived from the dictionary's `index.json` title via `css_slug()`. This allows per-dictionary styling in Anki (e.g. different colors for JE vs JJ dictionaries).

## Build & run

```sh
cargo build
cargo run                        # server on 0.0.0.0:3000
cargo run --bin tokenize -- "日本語のテキスト"  # test tokenizer output
cargo test                       # unit + integration (mocked)
cargo test -- --ignored          # real subprocess tests (need yt-dlp, whisper, Anki)
```

## Config

Configuration via environment variables. Loaded from `.env` automatically via `dotenvy`. Copy `.env.example` to `.env` and adjust.

| Variable | Default |
|---|---|
| `JP_TOOLS_DB_PATH` | `jp-tools.db` |
| `JP_TOOLS_AUDIO_DIR` | `audio` |
| `JP_TOOLS_MEDIA_DIR` | `media` |
| `JP_TOOLS_LISTEN_ADDR` | `0.0.0.0:3000` |
| `JP_TOOLS_TRANSCRIBE_SCRIPT` | `scripts/transcribe.py` |
| `JP_TOOLS_WHISPER_CPU_THREADS` | `0` (all cores) |
| `JP_TOOLS_WHISPER_DEVICE` | `auto` (`cpu`, `cuda`) |
| `JP_TOOLS_DICTIONARY_PATHS` | *(none, optional)* — comma-separated list of Yomitan zip files |
| `JP_TOOLS_DICTIONARY_PATH` | *(legacy)* — single path, fallback if `_PATHS` not set |

### Anki export config

Model name, deck name, and field mapping are configurable to match an existing Anki note type. Defaults match the "Japanese sentences" model used by Yomitan. If the model doesn't exist in Anki, a basic fallback is created. Set a field var to empty string to skip it.

| Variable | Default |
|---|---|
| `JP_TOOLS_ANKI_URL` | `http://localhost:8765` |
| `JP_TOOLS_ANKI_MODEL` | `Japanese sentences` |
| `JP_TOOLS_ANKI_DECK` | `Japanese` |
| `JP_TOOLS_ANKI_FIELD_VOCAB` | `VocabKanji` |
| `JP_TOOLS_ANKI_FIELD_DEFINITION` | `VocabDef` |
| `JP_TOOLS_ANKI_FIELD_SENTENCE` | `SentKanji` |
| `JP_TOOLS_ANKI_FIELD_IMAGE` | `Image` |
| `JP_TOOLS_ANKI_FIELD_AUDIO` | `SentAudio` |
| `JP_TOOLS_ANKI_FIELD_SOURCE` | `Document` |
| `JP_TOOLS_ANKI_FIELD_FURIGANA` | `VocabFurigana` |
| `JP_TOOLS_ANKI_FIELD_PITCH_NUM` | `VocabPitchNum` |

## Testing

- Unit tests: pure functions (URL validation, JSON parsing, status roundtrips)
- Integration tests: in-memory SQLite + `mockall` mocks for services
- Route tests: `axum-test::TestServer` with mocked dependencies
- `#[ignore]` tests: real subprocess calls for manual verification

## Next steps

- Improve target word selection UX: multi-token selection and/or editable target word field
- Fallback dictionary lookup (concatenated surface → base_form) for compound words
- Frequency-based filtering. See `spec/` for roadmap.
