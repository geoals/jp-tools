# yt-mine — YouTube Sentence Mining

Rust 2024 edition. Axum web server + htmx frontend, SQLite persistence.

## Pipeline

YouTube URL → yt-dlp download → faster-whisper transcription → Lindera tokenization → sentence display → target word selection → dictionary lookup → Anki export

Jobs run as background `tokio::spawn` tasks. Frontend polls via htmx fragments.

## Project structure

```
src/
  main.rs           — server bootstrap, wires concrete implementations
  lib.rs            — pub mod declarations
  app.rs            — AppState (DI container) + router
  config.rs         — env-based config (dotenvy)
  db.rs             — SQLite via sqlx, compile-time checked queries
  error.rs          — AppError enum → HTTP status mapping
  models.rs         — Job, JobStatus, Sentence, TranscriptSegment
  routes/mining.rs  — HTTP handlers, htmx polling, export orchestration
  services/
    pipeline.rs     — orchestrates download → transcribe → store
    download.rs     — AudioDownloader trait, YtDlpDownloader
    transcribe.rs   — Transcriber trait, WhisperWorker (persistent Python subprocess)
    export.rs       — AnkiExporter trait, AnkiConnectExporter
    llm.rs          — LlmDefiner trait, AnthropicDefiner
    media.rs        — MediaExtractor trait, FfmpegMediaExtractor
    tokenize.rs     — Tokenizer trait, LinderaTokenizer (UniDic)
    dictionary/
      mod.rs        — Dictionary loading, lookup, wrap_definitions
      html.rs       — structured_content_to_html (Yomitan JSON → HTML)
      tests.rs      — dictionary + HTML conversion tests
scripts/transcribe.py — faster-whisper persistent worker (stdin/stdout JSON)
templates/            — base layout + htmx fragments
```

## Key design decisions

- **Traits for external tools** — `AudioDownloader`, `Transcriber`, `AnkiExporter`, `MediaExtractor`, `Tokenizer`, `LlmDefiner` enable mocking via `mockall`
- **Subprocesses over FFI** — clean boundary for yt-dlp, faster-whisper, ffmpeg
- **Persistent whisper worker** — model loaded once, reused via JSON protocol
- **htmx over SPA** — server-rendered HTML fragments, no JS framework

## Tokenization

Lindera with UniDic. Morpheme-level tokens (短単位). Content-word POS filter: 名詞, 動詞, 形容詞, 形状詞, 副詞. Verb conjugations are split (known limitation).

## Dictionary lookup

Yomitan-format zips, exact headword match (HashMap). Multiple dictionaries concatenated with per-dictionary CSS classes (`dict-{slug}-title/body`). Pitch accent from `term_meta_bank_*.json`. Furigana uses Anki bracket notation (`食べる[たべる]`).

Structured-content JSON → HTML via recursive descent in `dictionary/html.rs`. Uses `data-content`/`data-class` attributes for CSS targeting.

## Build & run

```sh
cargo run -p yt-mine                              # server on 0.0.0.0:3000
JP_TOOLS_FAKE_API=true cargo run -p yt-mine       # dev mode (no external deps)
cargo test -p yt-mine                             # unit + integration (mocked)
cargo test -p yt-mine -- --ignored                # real subprocess tests
```

## Config

Via env vars, loaded from `.env` (repo root) via `dotenvy`. See `.env.example`.

Key variables: `JP_TOOLS_DB_PATH`, `JP_TOOLS_AUDIO_DIR`, `JP_TOOLS_MEDIA_DIR`, `JP_TOOLS_LISTEN_ADDR`, `JP_TOOLS_TRANSCRIBE_SCRIPT`, `JP_TOOLS_DICTIONARY_PATHS` (comma-separated Yomitan zips), `JP_TOOLS_FAKE_API`, `JP_TOOLS_ANTHROPIC_API_KEY`, `JP_TOOLS_LLM_MODEL`.

Anki export fields are all configurable via `JP_TOOLS_ANKI_*` vars (model, deck, field mapping). Defaults match "Japanese sentences" Yomitan note type.

## Testing

- Unit: pure functions (URL validation, JSON parsing, status roundtrips)
- Integration: in-memory SQLite + `mockall` mocks
- Route: `axum-test::TestServer` with mocked deps
- `#[ignore]`: real subprocess calls for manual verification
