# jp-tools — YouTube Sentence Mining

Rust (edition 2024) tool for mining Japanese sentences from YouTube videos. Axum web server with htmx frontend, SQLite persistence.

## Architecture

Pipeline: **YouTube URL → yt-dlp audio download → faster-whisper transcription → sentence storage → Anki export**

Jobs run as background `tokio::spawn` tasks. The frontend polls for status via htmx fragments.

### Key modules

- `src/services/pipeline.rs` — orchestrates the full job lifecycle
- `src/services/download.rs` — `AudioDownloader` trait, `YtDlpDownloader` (subprocess)
- `src/services/transcribe.rs` — `Transcriber` trait, `WhisperTranscriber` (Python subprocess)
- `src/services/export.rs` — `AnkiExporter` trait, `AnkiConnectExporter` (HTTP to localhost:8765)
- `src/routes/mining.rs` — HTTP handlers, htmx polling, form handling
- `src/db.rs` — SQLite via sqlx, compile-time checked queries
- `src/models.rs` — `Job`, `JobStatus` (state machine: Pending→Downloading→Transcribing→Done|Error), `Sentence`, `TranscriptSegment`
- `scripts/transcribe.py` — faster-whisper wrapper, outputs JSON to stdout

### Design decisions

- **Traits for external tools** — `AudioDownloader`, `Transcriber`, `AnkiExporter` are traits so tests can mock them via `mockall`
- **Subprocesses over FFI** — yt-dlp and faster-whisper are Python; subprocesses keep the boundary clean
- **htmx over SPA** — server-rendered HTML, no JS framework

## Build & run

```sh
cargo build
cargo run           # listens on 0.0.0.0:3000
cargo test          # unit + integration (mocked)
cargo test -- --ignored  # real subprocess tests (need yt-dlp, whisper, Anki)
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

## Testing

- Unit tests: pure functions (URL validation, JSON parsing, status roundtrips)
- Integration tests: in-memory SQLite + `mockall` mocks for services
- Route tests: `axum-test::TestServer` with mocked dependencies
- `#[ignore]` tests: real subprocess calls for manual verification

## Future (MVP 2+)

Morphological analysis (LinDera), audio clips per sentence, dictionary lookups, frequency-based filtering. See `spec/` for roadmap.
