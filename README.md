# jp-tools

Monorepo for Japanese language learning tools.

## Projects

- **[yt-mine/](yt-mine/)** — YouTube sentence mining: paste a YouTube URL, get transcribed sentences with tokenization, dictionary lookup, and one-click Anki export.
- **[manga-mine/](manga-mine/)** — physical manga sentence mining: photo inbox → crop → OCR → Anki, stateless (no database of its own).
- **[vn-mine/](vn-mine/)** — visual novel voiceline capture: single-hotkey audio + screenshot mining from a Textractor WebSocket hook, no in-game voice replay needed.
- **[read-stats/](read-stats/)** — daily reading tracker (chars/time) derived automatically from the vn-mine line stream, plus manual sessions for physical books.
- **[jp-core/](jp-core/)** and **[jp-mine-core/](jp-mine-core/)** — shared library crates: tokenization + dictionary lookup, and mining back-half (card formatting, AnkiConnect export).
- **[whisper-service/](whisper-service/)** — Python FastAPI transcription service backing yt-mine and vn-mine's sentence trim.
- **[manga-ocr-service/](manga-ocr-service/)** — Python FastAPI OCR service backing manga-mine.
- **[scripts/start-all.sh](scripts/start-all.sh)** — start/stop/status for the whole stack in one command.

## Specs

- **[spec/](spec/)** — original pre-implementation design docs. Superseded by the code and each project's own CLAUDE.md/README; kept for historical context on early design decisions. See [spec/index.md](spec/index.md).
