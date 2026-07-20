# jp-tools

Cargo workspace for Japanese language learning tools.

- `jp-core/` — tokenization (Sudachi, hybrid Mode C/B with dictionary validation) + dictionary (Yomitan zip parsing, SQLite cache) library crate
- `jp-mine-core/` — shared mining back half: dictionary lookup, card formatting, AnkiConnect export (used by yt-mine and manga-mine)
- `yt-mine/` — YouTube sentence mining (Axum JSON API + Preact SPA, SQLite, Anki export). See `yt-mine/CLAUDE.md`
- `manga-mine/` — physical manga sentence mining (photo inbox → crop → OCR → Anki, stateless). See `manga-mine/CLAUDE.md`
- `vn-mine/` — visual novel voiceline capture (bash/python, no Cargo member): audio ring-buffer daemon + clipboard-timestamp + silero-VAD hotkey script → Anki. See `vn-mine/README.md`
- `read-stats/` — daily reading tracker (Axum + SQLite + Preact, port 3200): chars/time derived from the line stream vn-ws-logger.py writes to `~/.local/share/jp-stats/stats.db`, plus manually logged sessions (mostly VN reading from before auto-tracking existed). Also serves `#read`, the phone-side live line feed + mine button used for reading a VN over Moonlight. See `read-stats/README.md`
- `manga-ocr-service/` — Python FastAPI wrapper around kha-white's manga-ocr (port 8200)
- `whisper-service/` — Python FastAPI transcription service for yt-mine (port 8100)
- `spec/` — feature specs and roadmap
- `scripts/start-all.sh` — start/stop/restart/status for the full stack (whisper-service, manga-ocr-service, yt-mine, manga-mine, read-stats); takes service names to act on just one (`restart read-stats`); see `--help`

```sh
cargo build              # all members
cargo test               # all members
cargo run -p yt-mine     # server on :3000
cargo run -p manga-mine  # server on :3100
```

## Working here

- Commit straight to `master`. This is a solo repo — don't create a feature
  branch for a change unless asked.
- In the Preact/htm SPAs (`read-stats`, `yt-mine`), never let literal text and
  `${...}` straddle a line break inside an ``html`` `` template. htm collapses
  the whitespace at the break, and prettier reflows markup there freely — that
  combination silently rendered `snapshot 0 min ago` as `snapshot0 minago`.
  Build the whole string in JS and interpolate it as one value:

  ```js
  const age = `snapshot ${mins} min ago`;   // then: <span>${age}</span>
  ```
