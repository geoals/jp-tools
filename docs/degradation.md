# What Kotodex needs, and what it does without it

One row per optional part: what it gives, what happens when it is missing, and
the one thing that turns it on. This is the specification for three
implementations of it — `kotodex-server/src/routes/reader/capabilities.rs`
(the probe), `setup.sh` (the installer) and `scripts/kotodex-doctor.sh` (`kotodex
doctor`) — so a change belongs in this table first.

Install commands are written generically; `scripts/lib/platform.sh` maps the
package names per distro.

## Required — nothing works without these

| part | gives | without it | turn it on |
|---|---|---|---|
| `python3` | the capture daemon, the VAD, the overlay shell | nothing starts | `pkg install python3` |
| `curl`, `jq` | every script's HTTP and JSON | capture and mining fail | `pkg install curl jq` |
| PySide6 + Qt WebEngine (**system** packages) | the overlay window and the page in it | no overlay; `#read` in a browser still works | `pkg install pyside6 qt6-webengine` — a pip/venv PySide6 has no `org.kde.layershell` |
| SudachiDict (`system_full.dic`) | tokenizing at all | no words, no lookups, no ledger | `setup.sh` downloads it (127 MB, Apache-2.0) |

## Audio capture

| part | gives | without it | turn it on |
|---|---|---|---|
| `pactl` | finds the sink to record | no voiceline audio on any card | `pkg install pulseaudio-utils` (or `pipewire-pulse`) |
| `ffmpeg` | the ring buffer, clip cutting, Vorbis encoding | no voiceline audio on any card | `pkg install ffmpeg` |
| silero VAD model (`silero_vad.onnx`) | trims the clip to the spoken line | the clip keeps its raw window — long, with room tone | `setup.sh` downloads it (2.2 MB, MIT) |
| `onnxruntime` (python) | runs the VAD model | same as a missing model | `setup.sh` installs it into `~/.local/share/kotodex/venv` |
| whisper-service | narrows the clip to the mined sentence | the VAD-trimmed clip is attached instead; the reader's trim indicator is off | not set up in this release — see `whisper-service/README.md` |

## Line source

| part | gives | without it | turn it on |
|---|---|---|---|
| Textractor WebSocket (`vn-ws-logger.py`) | the lines being read, live | no feed; the overlay shows the warning line and nothing else | run Textractor with the WS plugin pointed at the logger |
| clipboard watcher (`wl-clipboard` or `xclip`) | the same rows from a clipboard hooker | only matters as an alternative to the above | ⚙ → Source in the overlay, or `line_source` in settings |

## Dictionaries

Roles, not titles: any dictionary answers the wordhood gate, `standard` and
`master` decide segmentation, `master` alone is the vocabulary scale.

| part | gives | without it | turn it on |
|---|---|---|---|
| any definition dictionary | the popup has definitions | the popup opens empty; lines, marks and mining still work | `setup.sh` offers Jitendex (CC-BY-SA), or drop a zip in `dictionaries/` and re-run |
| master dictionary (`Role::Master`) | the vocabulary scale, and the tightest segmentation | the scale is not offered; segmentation falls back to whatever `standard` dictionaries exist | `jp-dict set-role <id> master` |
| `standard` monolinguals | word boundaries the master alone gets wrong | more over- and under-joining | `jp-dict set-role <id> standard` |
| frequency dictionary (`Role::Frequency`) | the common-word underline, the rank pill, by-frequency ordering | no underline, no pill, unordered sweep | `jp-dict import <zip>` — a jpdb-style list ranks fiction best |
| second frequency dictionary | the second rank pill | one pill instead of two | as above |
| pitch dictionary (`Role::Pitch`) | the accent line in the popup and on the card | no accent shown | `jp-dict import <zip>` |
| vocabulary ledger rows | status tints in the overlay, the dashboard's curves | tints off (opt-in anyway on a fresh install), empty-state dashboards | fills itself as you read; `POST /api/vocab/rebuild` re-derives it |

## Anki

| part | gives | without it | turn it on |
|---|---|---|---|
| Anki + AnkiConnect on `127.0.0.1:8765` | mining at all | the mine control is not drawn; reading and lookups are unaffected | install Anki, add the AnkiConnect add-on, leave Anki running |
| the note type and its fields | a card that renders | mining errors out per card | `kotodex anki check` reports what is missing and can create Lapis |
| screenshot tool (`spectacle`, `grim`, `gnome-screenshot`) | the still on the card | card without a picture | `pkg install spectacle` (KDE) / `grim` (wlroots) |
| `xdotool` + `import` | screenshots the *game* window rather than the focused one | falls back to the focused window, which may be the browser | `pkg install xdotool imagemagick` |

## Overlay

| part | gives | without it | turn it on |
|---|---|---|---|
| `layer-shell-qt` (`org.kde.layershell`) | the feed above a fullscreen game, click-through | falls back to the X11 backend, which is above a fullscreen game everywhere except KDE | `pkg install layer-shell-qt` — system package only |
| a compositor that stacks it | the same, in practice | see `docs/compositors.md` for which do | use KDE or a wlroots compositor for fullscreen |
| `xdotool` | tracks the game window's geometry so the strip follows it | the strip sits at a fixed screen position | `pkg install xdotool` |

## Extras

| part | gives | without it | turn it on |
|---|---|---|---|
| Anthropic API key | the ℹ explain button | the button is not drawn | `setup.sh` prompts, or set `KOTODEX_ANTHROPIC_API_KEY` |
