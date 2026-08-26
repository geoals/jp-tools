# manga-mine — Physical Manga Sentence Mining

Rust 2024 edition. Axum JSON API + Preact frontend (no build step), **and no
database of its own**: the inbox folder *is* the queue, the finished card lives
in Anki, and mined/skipped state is a file move into `processed/` or `skipped/`.
Re-mining is moving the file back. The server is stateless — crop coordinates and
OCR text are transient.

## Pipeline

Phone photo → inbox folder (synced/uploaded) → manual crop → manga-ocr-service →
jp-core tokenization → target word tap → dictionary lookup → Anki export (jp-mine-core)

## Statelessness

- **The inbox folder is the queue** — every image file in `KOTODEX_MANGA_INBOX`
  is an un-mined photo. Marking a photo mined/skipped **deletes** it: the
  original lives in the phone gallery and the compressed copy lives in Anki, so
  the server keeps nothing.
- The finished card lives in Anki (image via `storeMediaFile`; the temp
  compressed copy in `KOTODEX_MEDIA_DIR` is removed after export).
- Crop coordinates and OCR text are transient — nothing outlives the request.
- Remembered manga titles (the card's Document/source field) live in
  `<inbox>/.sources.json`, most-recent-first; `GET /api/sources` serves them
  and the UI preselects the latest.
- The dictionary cache lives in the shared `knowledge.db`
  (`KOTODEX_KNOWLEDGE_DB_PATH`); manga-mine only *reads* it. `jp-dict` imports.

## Key design decisions

- **Crop coordinates are fractions (0–1) of the *oriented* image.** Browsers
  render EXIF rotation applied; `image_ops` applies the same orientation before
  cropping so pixels match what the user drew on.
- **Card image = whole photo (compressed, max 1280px, q80 — configurable); the
  crop feeds OCR only.** The inbox keeps the original full-res photo;
  only the Anki copy is compressed.
- **Client AnkiConnect detection** — on export, the server probes the
  *requesting client's* IP on port 8765 (800 ms timeout). If the phone runs its
  own AnkiConnect, the card lands in the phone's collection; otherwise the
  configured `KOTODEX_ANKI_URL` is used. Loopback clients skip the probe.
  Disable with `KOTODEX_ANKI_USE_CLIENT=false`.
- **No audio, ever** — `audio_clip_path` is always `None`.
- **Export dedup is Anki's** — AnkiConnect rejects a note whose first field
  (VocabKanji) already exists; surfaced as an export error.
- Traits (`OcrEngine`, `AnkiExporter`, `Tokenizer`) enable mockall route tests.
- Exported notes are tagged `manga-mine, manga`.

## Build & run

```sh
cargo run -p manga-mine                           # server on 0.0.0.0:3100
KOTODEX_FAKE_API=true cargo run -p manga-mine    # dev mode (no external deps)
cargo test -p manga-mine

# OCR service (required in real mode):
cd manga-ocr-service && .venv/bin/uvicorn main:app --host 0.0.0.0 --port 8200
```

Requires Anki + AnkiConnect running for export; dictionaries and Sudachi dict
configured as for yt-mine (same env vars).

## Config

Env vars, names and defaults in `config.rs`. The `KOTODEX_ANKI_*` note
type/deck/field mapping is shared with yt-mine — same note type, and now the
same fields kotodex-server writes (`VocabDefFull`, `VocabPitchNum` +
`VocabPitchPattern`, `CompactDef`). The card's fields are built by
`jp_mine_core::card`. manga-mine has no LLM configured, so `CompactDef` is left
empty.
