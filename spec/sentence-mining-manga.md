# Sentence Mining from Physical Manga

Frictionless sentence mining from paper manga (and physical books/light novels):
photograph a panel while reading, then later turn each photo into a rich Anki
card — target word, example sentence, definition, panel image — with the same
low friction as mousing over a word in a digital reader.

## Problem

Digital immersion has a solved mining loop: hover a word, hit one key, get a
high-quality Anki card (word, sentence, audio, screenshot, definition). Physical
manga has none of this. The reader hits an unknown word, and the options are all
bad: stop reading to look it up and hand-build a card (kills flow and rarely
happens), or note it down to process later (tedious, error-prone transcription
of vertical, stylized Japanese).

The result is that reading physical manga — often the most enjoyable, immersive
input — produces almost no mined cards. The material that motivates study is the
material that feeds Anki the least.

What's needed is a loop that preserves reading flow at capture time and pushes
all the mechanical work (OCR, tokenization, dictionary lookup, card formatting)
into a later, batched pass.

## Design Principle: decouple capture from mining

The core insight: **capture and mining are separate acts.**

- **Capture** happens mid-reading and must cost nothing: point the phone's native
  camera at the panel, snap, keep reading. No app to open, no decision to make.
  The photo auto-syncs to the computer.
- **Mining** happens later, batched, at the desk (with Anki running): work
  through the captured photos, crop the text, pick the word, get a card.

Never OCR or mine live — that would wreck the reading flow, which is the entire
value of reading paper in the first place.

This mirrors `yt-mine`: the front of the pipeline differs (photo/OCR instead of
YouTube/whisper), but everything from tokenization onward is shared.

## Relationship to yt-mine

`manga-mine` is `yt-mine` with the front of the pipeline swapped. The back half
— tokenize → dictionary lookup → format card → export to Anki — is identical and
is extracted into a shared crate so both tools build on it. See ADR-001.

```
yt-mine:     URL   → yt-dlp → whisper → tokenize → pick word → dict lookup → Anki
manga-mine:  photo → crop   → OCR     → tokenize → pick word → dict lookup → Anki
                                        └──────────── jp-mine-core ─────────────┘
```

## Workflow Overview

```
Reading (paper manga)
│  native camera — snap a panel, keep reading
▼
┌──────────────┐     ┌──────────────────┐
│  Phone photo │────▶│  Sync to computer │   (auto, native photo sync)
└──────────────┘     └────────┬─────────┘
                              │
                              ▼
                     ┌──────────────────┐    also: upload an existing
                     │  Inbox (watched  │◀── photo from the phone gallery
                     │  folder) → queue │    (webapp over LAN)
                     └────────┬─────────┘
                              │  open webapp, work the queue
                              ▼
                     ┌──────────────────┐
                     │  Crop text region │   (manual box, every photo)
                     └────────┬─────────┘
                              │
                              ▼
                     ┌──────────────────┐
                     │  OCR the crop     │   (manga-ocr, local service)
                     └────────┬─────────┘
                              │
                              ▼
                     ┌──────────────────┐
                     │  Tokenize + split │   (jp-core; tappable words,
                     │  into sentences   │    sentence segmentation)
                     └────────┬─────────┘
                              │  tap the target word
                              ▼
                     ┌──────────────────┐
                     │  Dictionary lookup│   (jp-core: def, reading, pitch)
                     └────────┬─────────┘
                              │
                              ▼
                     ┌──────────────────┐
                     │  Export to Anki   │   (AnkiConnect, jp-mine-core)
                     │  sentence + word  │
                     │  + def + image    │
                     └──────────────────┘
```

## Card anatomy (v1)

One photo, one crop, one target word → one card:

| Field           | Source                                                              |
|-----------------|--------------------------------------------------------------------|
| Target word     | Tapped token; base form used for lookup                            |
| Example sentence| The single sentence (from the crop's OCR) containing the target    |
| Definition      | `jp-core` dictionary lookup — definition HTML, reading, pitch      |
| Image           | The **whole photo** (panel art + context), not the tight crop      |
| Audio           | *None* — manga cards are reading-recognition (ADR-005)             |

The Anki note type, field mapping, and export path are reused from `yt-mine`
unchanged (`ExportSentence` / `NoteData` / `AnkiConnectExporter`).

## Architecture

New crate `manga-mine` (Axum JSON API + Preact SPA), plus a new local OCR
microservice, both following existing `jp-tools` patterns. No database in v1 —
see "Persistence" below.

```
jp-core/            tokenization + dictionary            (exists, reused)
jp-mine-core/       lookup → format → Anki export        (NEW — extracted from yt-mine)
yt-mine/            YouTube front-end + jp-mine-core      (refactored onto shared core)
manga-mine/         photo/crop/OCR front-end + jp-mine-core   (NEW, stateless)
manga-ocr-service/  FastAPI wrapper around manga-ocr     (NEW — mirrors whisper-service)
```

- **`jp-mine-core`** — extracted shared logic: `lookup_word`, the card view/format
  types, and all of `export.rs` (`ExportSentence`, `AnkiExporter`, `NoteData`,
  `AnkiConfig`, `AnkiConnectExporter`). These are already domain-neutral in
  `yt-mine`; `ExportSentence` already carries `screenshot_path`, `target_word`,
  `definition`, `sentence_html`, etc.
- **`OcrEngine` trait** — the OCR seam, mirroring `Transcriber`/whisper-service.
  Default impl `MangaOcrEngine` posts a crop to `manga-ocr-service` and gets back
  text. Mockable for tests.
- **`manga-ocr-service`** — Python FastAPI container wrapping kha-white's
  `manga-ocr` (Transformer trained on manga; handles vertical text, ignores
  furigana). `POST /ocr { image crop } → { text }`. Recognition only — it expects
  a pre-cropped text region, which is why cropping is a mandatory manual step.
  This is the v1 impl behind `OcrEngine` (ADR-011). The upgrade path is **`owocr`**
  — the community-standard OCR multiplexer that fronts ~15 backends including
  **Google Lens** (often better than `manga-ocr` on hard manga) and `manga-ocr`
  bundled with **`comic-text-detector`** (the auto bubble detection deferred from
  ADR-004). `owocr` is daemon/socket/websocket-oriented (no one-shot HTTP), so a
  future `OwocrEngine` bridges it over its unix socket / websocket rather than a
  plain request. The `OcrEngine` trait keeps that swap cheap.
- **Ingestion** — a folder watcher on the synced inbox turns new photos into
  queue items; the webapp also accepts uploads of existing photos from the phone
  gallery (plain `<input type="file" accept="image/*">`, no live camera). The
  server is reachable from the phone over the LAN.
- **Persistence** — none. No database in v1 (ADR-010). The **inbox folder is the
  queue** (its contents = un-mined photos); the finished card lives in **Anki**
  (image sent via AnkiConnect `storeMediaFile`). Mined/skipped state is a **file
  move** into `processed/` or `skipped/` subfolders; re-mining = move the file
  back. The server is stateless; crop coordinates and OCR text are transient.

## MVP Phases

### MVP 1 — Photo → rich Anki card

The smallest useful thing: point at a synced photo, crop the text, pick the word,
send a card to Anki. This is the whole point — **screenshot → rich card.**

**Scope:**
- `jp-mine-core` extracted; `yt-mine` refactored onto it (no behavior change).
- `manga-ocr-service` running locally; `OcrEngine` trait + `MangaOcrEngine`.
- `manga-mine` server + SPA:
  - Inbox folder watch → queue of un-mined photos; gallery upload from phone.
  - Photo view with a manual crop box.
  - Crop → OCR → OCR text shown.
  - Tokenize (jp-core) → tappable words; sentence-split; tap = target word.
  - Dictionary lookup → definition/reading/pitch.
  - Export to Anki via AnkiConnect: sentence (target's sentence) + word +
    definition + whole-photo image. No audio.
  - Mark photo mined/skipped by moving the file to `processed/` / `skipped/`.
  - No database — the inbox folder is the queue, Anki holds the result (ADR-010).

### MVP 2 — Better selection & multi-region

- Deconjugation display on tap (surface vs. base form) to make phone selection
  easier — mostly a UI surfacing of existing `jp-core` token output.
- Multiple crops per photo (a wide shot with several bubbles → several cards).
- Optional LLM definition (reuse `yt-mine`'s optional `AnthropicDefiner`,
  off by default).

### MVP 3 — Polish

- Phone-first refinements (PWA install, LAN convenience).
- Robustness on the mining loop (skip/undo, malformed OCR handling).

### Later (out of v1..v3 scope)

- **Shared vocabulary tracking** across `yt-mine` and `manga-mine` (known/mining
  states, i+1 hints, don't-re-mine-known-words). Lives in `jp-mine-core` when
  built (ADR-009).
- **`owocr` backend** behind `OcrEngine` for **Google Lens** quality and, via its
  bundled **`comic-text-detector`**, **auto bubble detection** that replaces or
  augments manual cropping (ADR-004, ADR-011).
- **Queue + deferred Anki flush** for true mine-anywhere without desktop Anki
  running (ADR-008).

**Explicitly not planned:** audio / TTS of any kind — manga cards are reading-
recognition only (ADR-005).

## Glossary

- **Capture** — taking a photo of a panel mid-reading; deferred and non-blocking.
  The one act that must never interrupt reading.
- **Mining / Processing** — the later batched pass over captured photos that
  produces Anki cards.
- **Inbox** — the watched folder that phone photos sync into; new files become
  queue items.
- **Queue item** — one captured photo awaiting mining (new → mined / skipped).
- **Crop** — the user-drawn rectangle on a photo; the exact region sent to OCR and
  the source of the card's example sentence.
- **Target word** — the tapped token being studied; its dictionary base form
  drives lookup.
- **`jp-mine-core`** — new crate holding the shared back half (lookup → format →
  Anki export) used by both `yt-mine` and `manga-mine`.
- **`manga-ocr-service`** — local FastAPI service wrapping `manga-ocr`; does
  recognition on a pre-cropped region, not detection. The v1 `OcrEngine` backend.
- **`owocr`** — community-standard OCR multiplexer (Google Lens, `manga-ocr` +
  `comic-text-detector`, etc.); the deferred upgrade path behind `OcrEngine`.

## Decision Log (ADRs)

- **ADR-001 — Shared core + new crate.** Extract `lookup_word` and `export.rs`
  into `jp-mine-core`; `yt-mine` and `manga-mine` both build on it.
  *Trade-off:* upfront `yt-mine` refactor vs. long-term isolation of YouTube vs.
  manga concerns and no duplicated glue.
- **ADR-002 — Ingestion = folder-watch (primary) + gallery upload.** Native camera
  captures and auto-syncs into a watched inbox; the webapp also accepts uploads of
  existing photos from the phone. No live in-app camera in v1.
  *Trade-off:* two ingestion paths, but both are cheap and neither needs camera
  APIs.
- **ADR-003 — OCR = local `manga-ocr` behind an `OcrEngine` trait.** Manga-
  specialized, free, offline; mirrors `whisper-service`/`Transcriber`.
  *Consequence:* recognition-only, so a cropping step is mandatory (ADR-004).
- **ADR-004 — Manual crop on every photo.** User always drags a box; that crop is
  the sole OCR input. *Trade-off:* one gesture per card, but deterministic and no
  detection model / whole-image edge cases in v1.
- **ADR-005 — No audio, period.** Manga cards are reading-recognition; there is no
  source audio and synthetic TTS is explicitly out of scope (not deferred).
  *Rationale:* TTS adds a service and card-field complexity for value the user does
  not want on manga cards.
- **ADR-006 — Card image = whole photo; crop feeds OCR only.** Panel context beats
  a bare text-bubble image.
- **ADR-007 — Sentence-split the crop; card sentence = the sentence containing the
  target word.** Full crop text retained. Tight, focused cards (standard mining
  practice).
- **ADR-008 — Direct AnkiConnect only.** Reuse `yt-mine`'s exporter unchanged;
  mining happens at the desk with Anki running. *Trade-off:* no mine-anywhere
  without a running desktop Anki, in exchange for zero new export code.
- **ADR-009 — Vocab tracking deferred; belongs in `jp-mine-core` when built.** v1
  `manga-mine` does no known/unknown filtering. *Note:* superseded on the storage
  detail by ADR-010 — there is no `manga-mine` DB in v1; a shared vocab store, when
  built, lives in `jp-mine-core`.
- **ADR-010 — No database in v1; stateless server.** The inbox folder is the queue
  (its contents = un-mined photos); the finished card lives in Anki (image via
  AnkiConnect `storeMediaFile`); mined/skipped state is a file move into
  `processed/` / `skipped/`. *Rationale:* nothing in the crop→card loop needs to
  outlive the request — crop coords and OCR text are transient, and Anki + the
  filesystem are already the sources of truth. Adding SQLite would be state with no
  owner. *Trade-off:* no queryable history/analytics until a DB is introduced.
- **ADR-011 — OCR backend: thin `manga-ocr` service in v1, `owocr` as the upgrade
  path.** v1 `MangaOcrEngine` is a small FastAPI over the `manga_ocr` package
  (clean crop→text request/response). `owocr` — despite bundling Google Lens and
  `comic-text-detector` — is daemon/socket/websocket-oriented with no one-shot HTTP
  API and a folder-scan model that collides with manual cropping (ADR-004), so it's
  deferred to a future `OwocrEngine` behind the same trait. *Trade-off:* forgo
  Google Lens quality and bundled detection now, in exchange for a simple,
  synchronous, fully-local v1.
