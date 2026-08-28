# Kotodex / コトデックス

**A reading log for Japanese.** Every line you read and every word you look up,
kept locally — and what you know, word by word, derived from that rather than
from self-report.

On Linux it also reads visual novels: the line, its dictionary and your
vocabulary drawn over the game, fullscreen included, and one click mines an
Anki card with the character's voice on it.

## What it does

- **Counts what you read** — characters, hours, lookups, vocabulary over time.
  All of it derived from the raw line stream, so changing a threshold re-reads
  your whole history under the new rule.
- **Marks what you know** on the text as you read it.
- **Draws the line over the game.** A layer surface above a fullscreen window,
  click-through everywhere it has not drawn, following the game as it moves.
- **Mines a card with the voiceline.** A ring buffer is always recording, so the
  clip comes from audio that already happened; silero-VAD trims it to the spoken
  line and a screenshot goes on the card.

The log is the product and reading a VN here is one way to feed it. Text arrives
through one endpoint, `POST /api/lines`, so whatever captures it can be another
tool, another machine, or a phone — see [sources/README.md](sources/README.md).

## Install

```
curl -fsSL https://raw.githubusercontent.com/geoals/kotodex/master/install.sh | sh
```

Or from a release tarball:

```
tar xf kotodex-<version>-linux-x86_64.tar.gz && cd kotodex-<version>
./setup.sh
```

It checks what is missing, prints the install line for your distro, and is a
no-op for whatever is already done. Re-run it any time.

`--core` installs the log and the reader alone — no Qt, no audio, none of the
parts that only exist to read a VN on this machine.

On **Windows**, run `kotodex-<version>-windows-setup.exe` from a release. It
installs per-user, so there is no elevation prompt, and it has no card audio or
game screenshot — see [docs/install.md](docs/install.md).

## Requirements

- **PySide6 and Qt WebEngine as system packages.** A pip PySide6 has no
  `org.kde.layershell`.
- **Textractor** with its WebSocket plugin, for the lines.
- **Anki with AnkiConnect**, for cards. Everything else works without it.

Compositors: [docs/compositors.md](docs/compositors.md). What each optional part
adds and what happens without it: [docs/degradation.md](docs/degradation.md), or
`kotodex doctor` for this machine.

## Dictionaries

Kotodex ships none. Drop Yomitan zips into `dictionaries/` and re-run
`setup.sh`; `jp-dict` reads what each one holds and gives it a role —
definitions, frequency, pitch.

One is the **master**: a monolingual dictionary whose headword list decides what
counts as a word. Your vocabulary count is how many of *its* headwords you have
marked known, so adding dictionaries changes what you can look up and never the
denominator. 三省堂国語辞典 lists ~82k headwords; Jitendex holds 335k more —
variants, compounds, phrases — and a count against those would mean nothing.

## Also in this repository

Kotodex is `kotodex/` (the launcher), `kotodex-server/` (the server, the
dashboard and the overlay page), `layer-overlay/` (the Qt shell that puts a web
page over a fullscreen window), `capture/` (the audio and screenshot a card
needs) and `sources/` (what hands captured text to the log).

Not part of the release, but sharing the same language layer:

- **[yt-mine/](yt-mine/)** — YouTube sentence mining: a URL in, transcribed and
  tokenized sentences out, one-click Anki export.
- **[manga-mine/](manga-mine/)** — physical manga: photo inbox → crop → OCR →
  Anki.
- **[jp-core/](jp-core/)**, **[jp-mine-core/](jp-mine-core/)** — tokenizing,
  dictionaries and the vocabulary ledger; the card's fields and its gloss.
- **[whisper-service/](whisper-service/)**, **[manga-ocr-service/](manga-ocr-service/)**
  — the two Python services those need.
- **[scripts/start-all.sh](scripts/start-all.sh)** — start, stop or restart every
  service in the repository, or one by name. A development tool: Kotodex runs
  what it needs and is not in here.

## Licence

GPL-3.0. [THIRD-PARTY.md](THIRD-PARTY.md) lists what other people wrote. Each
directory's own `CLAUDE.md` or `README.md` is its documentation.
