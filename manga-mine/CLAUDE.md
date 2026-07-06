# manga-mine — Physical Manga Sentence Mining

Rust 2024 edition. Axum JSON API + Preact frontend (no build step), **no database**
(ADR-010 in `spec/sentence-mining-manga.md`).

## Pipeline

Phone photo → inbox folder (synced/uploaded) → manual crop → manga-ocr-service →
jp-core tokenization → target word tap → dictionary lookup → Anki export (jp-mine-core)

## Statelessness

- **The inbox folder is the queue** — every image file in `JP_TOOLS_MANGA_INBOX`
  is an un-mined photo. Marking a photo mined/skipped **deletes** it (amends
  ADR-010's file-move): the original lives in the phone gallery, the compressed
  copy lives in Anki, so the server keeps nothing.
- The finished card lives in Anki (image via `storeMediaFile`; the temp
  compressed copy in `JP_TOOLS_MEDIA_DIR` is removed after export).
- Crop coordinates and OCR text are transient — nothing outlives the request.
- Remembered manga titles (the card's Document/source field) live in
  `<inbox>/.sources.json`, most-recent-first; `GET /api/sources` serves them
  and the UI preselects the latest.
- The dictionary cache SQLite DB (`JP_TOOLS_DB_PATH`, default `yt-mine.db`) is
  shared with yt-mine; manga-mine only reads/imports dictionaries there.

## Project structure

```
src/
  main.rs           — server bootstrap, wires concrete implementations
  app.rs            — AppState (DI container) + router, SPA shell handler
  config.rs         — env-based config (dotenvy)
  error.rs          — AppError enum → HTTP status mapping
  text.rs           — sentence segmentation for OCR text
  routes/api.rs     — JSON API (queue, photo/thumb, upload, ocr, preview, export, mark)
  services/
    ocr.rs          — OcrEngine trait, MangaOcrEngine (manga-ocr-service client)
    image_ops.rs    — EXIF-aware crop / compress / thumbnail (spawn_blocking)
    fake.rs         — fakes for dev mode (JP_TOOLS_FAKE_API=true)
static/               — Preact components (queue page, crop box, photo page)
templates/spa.html    — HTML shell (inlined via include_str!)
```

## Key design decisions

- **Crop coordinates are fractions (0–1) of the *oriented* image.** Browsers
  render EXIF rotation applied; `image_ops` applies the same orientation before
  cropping so pixels match what the user drew on.
- **Card image = whole photo (compressed, max 1280px, q80 — configurable); the
  crop feeds OCR only** (ADR-006). The inbox keeps the original full-res photo;
  only the Anki copy is compressed.
- **Client AnkiConnect detection** — on export, the server probes the
  *requesting client's* IP on port 8765 (800 ms timeout). If the phone runs its
  own AnkiConnect, the card lands in the phone's collection; otherwise the
  configured `JP_TOOLS_ANKI_URL` is used. Loopback clients skip the probe.
  Disable with `JP_TOOLS_ANKI_USE_CLIENT=false`.
- **No audio, ever** (ADR-005) — `audio_clip_path` is always `None`.
- **Export dedup is Anki's** — AnkiConnect rejects a note whose first field
  (VocabKanji) already exists; surfaced as an export error.
- Traits (`OcrEngine`, `AnkiExporter`, `Tokenizer`) enable mockall route tests.

## Build & run

```sh
cargo run -p manga-mine                           # server on 0.0.0.0:3100
JP_TOOLS_FAKE_API=true cargo run -p manga-mine    # dev mode (no external deps)
cargo test -p manga-mine

# OCR service (required in real mode):
cd manga-ocr-service && .venv/bin/uvicorn main:app --host 0.0.0.0 --port 8200
```

Requires Anki + AnkiConnect running for export; dictionaries and Sudachi dict
configured as for yt-mine (same env vars).

## Config

`JP_TOOLS_MANGA_INBOX` (default `manga-inbox`), `JP_TOOLS_MANGA_LISTEN_ADDR`
(default `0.0.0.0:3100`), `JP_TOOLS_OCR_SERVICE_URL` (default
`http://localhost:8200`), `JP_TOOLS_ANKI_USE_CLIENT` (default `true`),
`JP_TOOLS_MANGA_CARD_IMAGE_MAX_DIM` (default `1280`),
`JP_TOOLS_MANGA_CARD_IMAGE_QUALITY` (default `80`), plus the shared vars: `JP_TOOLS_DB_PATH`,
`JP_TOOLS_MEDIA_DIR`, `JP_TOOLS_DICTIONARY_PATHS`, `JP_TOOLS_SUDACHI_DICT_PATH`,
`JP_TOOLS_ANKI_*` (note type/deck/field mapping — same note type as yt-mine).
Exported notes are tagged `manga-mine, manga`.
