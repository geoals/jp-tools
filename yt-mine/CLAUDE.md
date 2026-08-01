# yt-mine — YouTube Sentence Mining

Rust 2024 edition. Axum JSON API + Preact frontend (no build step), SQLite persistence.

## Pipeline

YouTube URL → yt-dlp download → whisper-service transcription → Sudachi tokenization (Mode C + dictionary validation) → sentence display → target word selection → dictionary lookup → Anki export

Jobs run as background `tokio::spawn` tasks. Frontend polls via JSON API.

## Key design decisions

- **Tokenizer + dictionary in `jp-core`** — shared library crate with Sudachi tokenization (hybrid Mode C/B: Mode C for compounds, validated against dictionary headwords, unknown compounds split to Mode B) and Yomitan dictionary parsing
- **Traits for external tools** — `AudioDownloader`, `Transcriber`, `AnkiExporter`, `MediaExtractor`, `Tokenizer` (in jp-core), `LlmDefiner` enable mocking via `mockall`
- **Subprocesses over FFI** — clean boundary for yt-dlp, ffmpeg
- **Remote whisper-service** — transcription offloaded to separate FastAPI container (NDJSON streaming)
- **Preact + htm + signals from CDN** — no build step, ES module imports from esm.sh with pinned versions
- **JSON API + SPA shell** — `/api/*` returns JSON, `/` and `/{video_id}` serve the SPA shell

## Not built

Smart filtering — frequency thresholds, i+1 sentence selection, dimming
sentences whose words are all known. It would filter against the
`vocabulary` ledger in `knowledge.db` (owned by read-stats), which yt-mine
does not read today.

## Tokenization & Dictionary

Provided by `jp-core` crate. See `jp-core/` for details.

- Sudachi hybrid Mode C/B: tokenizes with Mode C, keeps compounds that exist as dictionary headwords (天気予報, 自己紹介), splits unknown compounds to Mode B sub-morphemes. Falls back to pure Mode B when no dictionaries are loaded
- Yomitan-format zips, exact headword match, pitch accent, structured-content HTML

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

Anki export fields are all configurable via `JP_TOOLS_ANKI_*` vars (model, deck, field mapping). Defaults match "Japanese sentences" Yomitan note type.
