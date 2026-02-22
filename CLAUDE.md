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
    dictionary.rs     — Dictionary struct, loads Yomitan zip files, exact-match lookup
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

Loads a Yomitan-format zip file. Lookup is exact match on headword (HashMap). The target word sent from the UI is the token's `base_form` (dictionary form from UniDic). If no match, VocabDef is left empty on the Anki card.

## Build & run

```sh
cargo build
cargo run                        # server on 0.0.0.0:3000
cargo run --bin tokenize -- "日本語のテキスト"  # test tokenizer output
cargo test                       # unit + integration (mocked)
cargo test -- --ignored          # real subprocess tests (need yt-dlp, whisper, Anki)
```

## Config (env vars)

| Variable | Default |
|---|---|
| `JP_TOOLS_DB_PATH` | `jp-tools.db` |
| `JP_TOOLS_AUDIO_DIR` | `audio` |
| `JP_TOOLS_LISTEN_ADDR` | `0.0.0.0:3000` |
| `JP_TOOLS_ANKI_URL` | `http://localhost:8765` |
| `JP_TOOLS_TRANSCRIBE_SCRIPT` | `scripts/transcribe.py` |
| `JP_TOOLS_WHISPER_CPU_THREADS` | `0` (all cores) |
| `JP_TOOLS_WHISPER_DEVICE` | `auto` (`cpu`, `cuda`) |
| `JP_TOOLS_MEDIA_DIR` | `media` |
| `JP_TOOLS_DICTIONARY_PATH` | *(none, optional)* |

## Testing

- Unit tests: pure functions (URL validation, JSON parsing, status roundtrips)
- Integration tests: in-memory SQLite + `mockall` mocks for services
- Route tests: `axum-test::TestServer` with mocked dependencies
- `#[ignore]` tests: real subprocess calls for manual verification

## Next steps

- Improve target word selection UX: multi-token selection and/or editable target word field
- Fallback dictionary lookup (concatenated surface → base_form) for compound words
- Frequency-based filtering. See `spec/` for roadmap.
