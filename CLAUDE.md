# jp-tools

Cargo workspace for Japanese language learning tools.

- `jp-core/` — tokenization (Sudachi, hybrid Mode C/B with dictionary validation) + dictionary (Yomitan zip parsing, SQLite cache) library crate
- `jp-mine-core/` — shared mining back half: dictionary lookup, card formatting, AnkiConnect export (used by yt-mine and manga-mine)
- `yt-mine/` — YouTube sentence mining (Axum JSON API + Preact SPA, SQLite, Anki export). See `yt-mine/CLAUDE.md`
- `manga-mine/` — physical manga sentence mining (photo inbox → crop → OCR → Anki, stateless). See `manga-mine/CLAUDE.md`
- `manga-ocr-service/` — Python FastAPI wrapper around kha-white's manga-ocr (port 8200)
- `whisper-service/` — Python FastAPI transcription service for yt-mine (port 8100)
- `spec/` — feature specs and roadmap

```sh
cargo build              # all members
cargo test               # all members
cargo run -p yt-mine     # server on :3000
cargo run -p manga-mine  # server on :3100
```
