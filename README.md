# Kotodex / コトデックス

Read a Japanese visual novel with the line, its dictionary and your own
vocabulary history drawn over the game — fullscreen included — and mine a card
with the character's voice on it in one click.

Linux only. The rest of this repository is the tooling it grew out of.

## What you get

- **The line over the game.** A layer surface above a fullscreen window, click-through
  everywhere it has not drawn. It follows the game window as it moves.
- **A dictionary popup on any word**, from your own Yomitan dictionaries —
  definitions, pitch accent, two frequency ranks, one dictionary at a time.
- **Anki cards with the voiceline.** A ring buffer is always recording, so the
  clip is cut from audio that already happened; silero-VAD trims it to the
  spoken line, and a screenshot of the game goes on the card.
- **What you know, painted on the text.** Every word you have judged is marked
  as you read, and a reading tracker behind it counts characters, hours,
  lookups and the size of your vocabulary over time.

## Requirements

- A compositor with `zwlr_layer_shell_v1` — KDE, Hyprland, any wlroots one —
  for the fullscreen overlay. Elsewhere it falls back to an always-on-top X11
  window, which works on GNOME under `QT_QPA_PLATFORM=xcb`. See
  [docs/compositors.md](docs/compositors.md).
- PySide6 and Qt WebEngine as **system** packages. A pip/venv PySide6 has no
  `org.kde.layershell`.
- Textractor with its WebSocket plugin, for the lines.
- Anki with AnkiConnect, if you want cards. Everything else works without it.

`setup.sh` checks all of this and prints the install line for your distro.

## Install

```
tar xf kotodex-<version>-linux-x86_64.tar.gz
cd kotodex-<version>
./setup.sh
```

It downloads SudachiDict and the VAD model, imports any dictionary zip you have
put in `dictionaries/`, and puts Kotodex in your application menu. Re-run it any
time — it is a no-op for whatever is already done.

`./setup.sh --dry-run` prints what it would do. `./setup.sh --uninstall` removes
it, and asks separately before touching your reading history.

## Dictionaries

Kotodex ships none: drop Yomitan zips into `dictionaries/` and re-run
`setup.sh`. `jp-dict` reads what each zip holds and gives it a role —
definitions, frequency, pitch — and the roles are what everything asks for, so
no dictionary is named anywhere in the code.

A monolingual master (三省堂, 明鏡) is what the vocabulary count is measured
against. [Jitendex](https://jitendex.org) is the free bilingual one. A
jpdb-style frequency list ranks fiction far better than a newspaper corpus
does.

## What degrades, and how

Nothing is required except a tokenizer dictionary and something to read.
[docs/degradation.md](docs/degradation.md) is one row per optional part: what it
gives, what happens without it, and the command that turns it on. `kotodex
doctor` prints your machine's version of that table.

## Also in this repository

Kotodex is `kotodex/` (the launcher), `read-stats/` (the server, the dashboard
and the overlay page), `layer-overlay/` (the Qt shell that puts a web page over
a fullscreen window) and `vn-mine/` (the capture daemon and the line logger).

Sharing the same language layer:

- **[yt-mine/](yt-mine/)** — YouTube sentence mining: a URL in, transcribed and
  tokenized sentences out, one-click Anki export.
- **[manga-mine/](manga-mine/)** — physical manga: photo inbox → crop → OCR →
  Anki.
- **[jp-core/](jp-core/)**, **[jp-mine-core/](jp-mine-core/)** — tokenizing,
  dictionaries and the vocabulary ledger; the card's fields and its gloss.
- **[whisper-service/](whisper-service/)**, **[manga-ocr-service/](manga-ocr-service/)**
  — the two Python services those two need.
- **[scripts/start-all.sh](scripts/start-all.sh)** — start, stop or restart the
  whole stack, or one service by name.

[THIRD-PARTY.md](THIRD-PARTY.md) lists what other people wrote. Each directory's
own `CLAUDE.md` or `README.md` is its documentation.
