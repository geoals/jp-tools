# Kotodex — first release

Read a Japanese visual novel with the line, its dictionary and your own
vocabulary history drawn over the game — fullscreen included — and mine a card
with the character's voice on it in one click. Linux only.

## Install

```
tar xf kotodex-<version>-linux-x86_64.tar.gz
cd kotodex-<version>
./setup.sh
```

The binaries are prebuilt; no Rust toolchain is needed. `setup.sh` checks the
dependencies and prints the install line for your distro, downloads SudachiDict
and the silero VAD model, offers Jitendex and the Jiten frequency list, checks
Anki and can install the Lapis note type, and installs the application entry.
It is re-runnable, and takes `--dry-run`, `--yes` and `--uninstall`.

## In this release

- **The overlay.** Layer surface where the compositor has `zwlr_layer_shell_v1`
  — KDE, Hyprland, wlroots — and an always-on-top X11 window elsewhere,
  including GNOME under `QT_QPA_PLATFORM=xcb`. Click-through everywhere it has
  not drawn.
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

- Windows and macOS are not supported and are not planned.
- Whisper (used by the YouTube tooling in this repository) is not set up by the
  installer.
- The GNOME X11 fallback is the least tested path; open an issue with your
  `kotodex doctor` output if the overlay does not stay above the game.

Licence: GPL-3.0. Bundled and downloaded assets are attributed in
[THIRD-PARTY.md](../THIRD-PARTY.md).
