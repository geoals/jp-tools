# yt-mine — YouTube Sentence Mining

Rust 2024 edition. Axum JSON API + Preact frontend (no build step), SQLite persistence.

## Pipeline

YouTube URL → yt-dlp download → whisper-service transcription → Lindera tokenization → sentence display → target word selection → dictionary lookup → Anki export

Jobs run as background `tokio::spawn` tasks. Frontend polls via JSON API.

## Project structure

```
src/
  main.rs           — server bootstrap, wires concrete implementations
  lib.rs            — pub mod declarations
  app.rs            — AppState (DI container) + router, SPA shell handler
  config.rs         — env-based config (dotenvy)
  db.rs             — SQLite via sqlx, compile-time checked queries
  error.rs          — AppError enum → HTTP status mapping
  models.rs         — Job, JobStatus, Sentence, TranscriptSegment
  routes/
    api/mod.rs      — JSON API handlers (submit, poll, preview, export, audio)
    api/tests.rs    — JSON API route tests
    mining/mod.rs   — shared business logic (tokenize, lookup, format)
    mining/tests.rs — pure function tests
  services/
    pipeline.rs     — orchestrates download → transcribe → store
    download.rs     — AudioDownloader trait, YtDlpDownloader
    transcribe.rs   — Transcriber trait, RemoteTranscriber (whisper-service client)
    export.rs       — AnkiExporter trait, AnkiConnectExporter
    llm.rs          — LlmDefiner trait, AnthropicDefiner
    media.rs        — MediaExtractor trait, FfmpegMediaExtractor
    fake.rs         — fake impls for dev mode (JP_TOOLS_FAKE_API=true)
static/               — Preact components, CSS, fetch wrappers, router, signals
templates/spa.html    — minimal HTML shell (inlined via include_str!)
```

## Key design decisions

- **Tokenizer + dictionary in `jp-core`** — shared library crate with Lindera/UniDic tokenization and Yomitan dictionary parsing
- **Traits for external tools** — `AudioDownloader`, `Transcriber`, `AnkiExporter`, `MediaExtractor`, `Tokenizer` (in jp-core), `LlmDefiner` enable mocking via `mockall`
- **Subprocesses over FFI** — clean boundary for yt-dlp, ffmpeg
- **Remote whisper-service** — transcription offloaded to separate FastAPI container (NDJSON streaming)
- **Preact + htm + signals from CDN** — no build step, ES module imports from esm.sh with pinned versions
- **JSON API + SPA shell** — `/api/*` returns JSON, `/` and `/{video_id}` serve the SPA shell

## Tokenization & Dictionary

Provided by `jp-core` crate. See `jp-core/` for details.

- Lindera with UniDic, morpheme-level tokens (短単位), content-word POS filter
- Yomitan-format zips, exact headword match, pitch accent, structured-content HTML

## Build & run

```sh
cargo run -p yt-mine                              # server on 0.0.0.0:3000
JP_TOOLS_FAKE_API=true cargo run -p yt-mine       # dev mode (no external deps)
cargo test -p yt-mine                             # unit + integration (mocked)
cargo test -p yt-mine -- --ignored                # real subprocess tests
```

## Config

Via env vars, loaded from `.env` (repo root) via `dotenvy`. See `.env.example`.

Key variables: `JP_TOOLS_DB_PATH`, `JP_TOOLS_AUDIO_DIR`, `JP_TOOLS_MEDIA_DIR`, `JP_TOOLS_LISTEN_ADDR`, `JP_TOOLS_WHISPER_SERVICE_URL`, `JP_TOOLS_DICTIONARY_PATHS` (comma-separated Yomitan zips), `JP_TOOLS_FAKE_API`, `JP_TOOLS_ANTHROPIC_API_KEY`, `JP_TOOLS_LLM_MODEL`.

Anki export fields are all configurable via `JP_TOOLS_ANKI_*` vars (model, deck, field mapping). Defaults match "Japanese sentences" Yomitan note type.

## Testing

- Unit: pure functions (URL validation, JSON parsing, status roundtrips)
- Integration: in-memory SQLite + `mockall` mocks
- Route: `axum-test::TestServer` with mocked deps
- `#[ignore]`: real subprocess calls for manual verification
