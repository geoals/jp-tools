# Kotodex

Read a Japanese visual novel with the line, its dictionary and your own
vocabulary history drawn over the game — fullscreen included — and mine a card
with the character's voice on it in one click.

## Install — Windows

Download `kotodex-<version>-windows-setup.exe` and run it. No administrator: it
installs under your own user directory, so there is no elevation prompt.

**Windows will say "Windows protected your PC".** The installer is not code-signed
— a certificate costs real money — so click *More info*, then *Run anyway*.

First run downloads about 175 MB: SudachiDict, and the dictionaries that are free
but not ours to redistribute. It needs a network connection once.

Two things the installer cannot do for you:

- **Textractor**, with its WebSocket extension enabled. That is what hooks the
  game's text, and without it the overlay stays empty. Kotodex listens on
  `ws://localhost:6677`; the address is in the settings panel if yours differs.
- **Anki** with the AnkiConnect add-on, if you want to mine cards. Reading and
  lookups work without it.

Then start Kotodex from the Start Menu. It starts the ledger, the line source and
the overlay, and opens the dashboard — where you say which work you are reading
and which window title the game has, so the overlay can follow it.

Audio on cards is Linux-only for now: a Windows card gets the sentence, the
definition and the frequency, without the voiceline.

## Install — Linux

```
tar xf kotodex-<version>-linux-x86_64.tar.gz
cd kotodex-<version>
./setup.sh
```

The binaries are prebuilt; no Rust toolchain is needed. `setup.sh` checks the
dependencies and prints the install line for your distro, downloads SudachiDict
and the silero VAD model, offers Jitendex, the Jiten frequency list and Kanjium pitch accents, checks
Anki and can install the Lapis note type, and installs the application entry.
It is re-runnable, and takes `--dry-run`, `--yes` and `--uninstall`.

## What it does

- **The overlay.** Layer surface where the compositor has `zwlr_layer_shell_v1`
  — KDE, Hyprland, wlroots — an always-on-top X11 window elsewhere, including
  GNOME under `QT_QPA_PLATFORM=xcb`, and a layered topmost window on Windows.
  Click-through everywhere it has not drawn.
- **The dictionary popup**, over your own Yomitan dictionaries: definitions,
  pitch accent, two frequency ranks, one dictionary at a time.
- **Scrollback**, paged back through the session with the same lookups and the
  same session dividers the dashboard draws.
- **Cards with the voiceline.** A ring buffer is always recording, silero-VAD
  trims the clip, and a screenshot of the game goes on the card.
- **The reading tracker** behind it: characters, hours, lookups and the size of
  your vocabulary over time, on a dashboard at `http://127.0.0.1:3200`.
- **Two line sources**, a Textractor WebSocket at a configurable address or the
  clipboard, switched in the settings panel without a restart.
- **Two Anki profiles**, `lapis` (default) and `legacy`, with every field name
  overridable through `KOTODEX_ANKI_FIELD_*`.

## What degrades, and how

Anything missing leaves a smaller working product and one sentence saying what
is off. No dictionaries: lines and history, no popup. No Anki: no mining
controls. No VAD model: cards without audio. `kotodex doctor` reports every
one of them, and [docs/degradation.md](degradation.md) is the full list.

## Known limits

- macOS is not supported and is not planned.
- **On Windows:** no card audio and no game screenshot, since the capture
  pipeline is Linux-only. A game in DirectX *exclusive* fullscreen cannot be
  overlaid at all — run it borderless windowed, which most engines do anyway.
  The dashboard's window picker lists nothing, so type the game's window title in
  by hand.
- Whisper (used by the YouTube tooling in this repository) is not set up by the
  installer.
- The GNOME X11 fallback is the least tested path; open an issue with your
  `kotodex doctor` output if the overlay does not stay above the game.

Licence: GPL-3.0. Bundled and downloaded assets are attributed in
[THIRD-PARTY.md](../THIRD-PARTY.md).
