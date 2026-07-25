# jp-tools

Cargo workspace for Japanese language learning tools.

- `jp-core/` — the language layer, shared by everything: `text` (character
  counting, the 「」 dialogue split, sentence segmentation), `tokenize`
  (Sudachi, hybrid Mode C/B with dictionary validation), `dictionary` (Yomitan
  zip parsing), and `knowledge` — the schema and handle for the shared
  `knowledge.db`
- `jp-mine-core/` — shared mining back half: dictionary lookup, card formatting, AnkiConnect export (used by yt-mine and manga-mine)
- `yt-mine/` — YouTube sentence mining (Axum JSON API + Preact SPA, SQLite, Anki export). See `yt-mine/CLAUDE.md`
- `manga-mine/` — physical manga sentence mining (photo inbox → crop → OCR → Anki, stateless). See `manga-mine/CLAUDE.md`
- `vn-mine/` — visual novel voiceline capture (bash/python, no Cargo member): audio ring-buffer daemon + clipboard-timestamp + silero-VAD hotkey script → Anki. See `vn-mine/README.md`
- `read-stats/` — daily reading tracker (Axum + SQLite + Preact, port 3200): chars/time derived from the line stream vn-ws-logger.py writes to `knowledge.db`, plus manually logged sessions (mostly VN reading from before auto-tracking existed). Also serves `#read`, the phone-side live line feed + mine button used for reading a VN over Moonlight. See `read-stats/CLAUDE.md`
- `manga-ocr-service/` — Python FastAPI wrapper around kha-white's manga-ocr (port 8200)
- `whisper-service/` — Python FastAPI transcription service for yt-mine (port 8100)
- `spec/` — feature specs and roadmap; `spec/knowledge-db.md` is the current
  architecture, not a proposal
- `scripts/start-all.sh` — start/stop/restart/status for the full stack (whisper-service, manga-ocr-service, yt-mine, manga-mine, read-stats); takes service names to act on just one (`restart read-stats`); see `--help`
- `scripts/vn.sh` — start/stop/restart/status for just the VN reading stack (read-stats + optional whisper-service), a thin wrapper over `start-all.sh` that also reports the `vn-buffer` systemd unit; `no-whisper` skips whisper. See `--help`
- `scripts/dev-instance.sh` — run read-stats in isolation (copy of the data,
  port 3299) and diff its endpoints before/after a change. Use this instead of
  restarting the live one.

```sh
cargo build              # all members
cargo test               # all members
cargo run -p yt-mine     # server on :3000
cargo run -p manga-mine  # server on :3100
```

## The databases

| file | holds | owner |
|---|---|---|
| `knowledge.db` | dictionary cache (+ role), `works`, `lines`, `manual_sessions`, `anki_notes`, `word_days`, `lookups` | `jp_core::knowledge` |
| `read-stats.db` | `settings`, `reader_marks`, `work_covers` | read-stats |
| `yt-mine.db` | `mining_jobs`, `mining_sentences` | yt-mine |

All under `~/.local/share/jp-tools/`. The split is by what the data *is*, not
by which app wrote it first: anything other tools will ask questions of —
what has been read, what has been looked up, what the dictionaries say — is
shared. `spec/knowledge-db.md` has the reasoning.

## Working here

- Commit straight to `master`. This is a solo repo — don't create a feature
  branch for a change unless asked.
- **Never restart the stack or touch `~/.local/share/jp-tools` while a VN is
  being read.** vn-ws-logger.py cannot be restarted while Textractor is
  attached to the game. `scripts/dev-instance.sh` exists so read-stats can be
  worked on regardless.
- In the Preact/htm SPAs (`read-stats`, `yt-mine`), never let literal text and
  `${...}` straddle a line break inside an ``html`` `` template. htm collapses
  the whitespace at the break, and prettier reflows markup there freely — that
  combination silently rendered `snapshot 0 min ago` as `snapshot0 minago`.
  Build the whole string in JS and interpolate it as one value:

  ```js
  const age = `snapshot ${mins} min ago`;   // then: <span>${age}</span>
  ```
