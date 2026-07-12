# vn-mine

Single-hotkey visual novel sentence mining: attach the last voiceline's audio
and a screenshot of the active window to the most recently added
"Japanese sentences" Anki note.

Works without any in-game voice replay: a daemon keeps the last 300s of
desktop audio in a tmpfs ring buffer and timestamps every Japanese line
Textractor copies to the clipboard — the copy moment marks the voiceline
start, and silero-VAD finds where the speech ends.

## Components

- `vn-buffer.sh` — daemon: ffmpeg ring buffer (60 × 5s WAV segments from the
  default sink monitor) + `wl-paste --watch` clipboard line logger, both in
  `$XDG_RUNTIME_DIR/vn-mine/`. Run via the `vn-buffer.service` systemd user
  unit.
- `vn-capture.sh` — bind to a hotkey. Screenshots the active window, cuts
  audio from the last hooked line's timestamp to the VAD speech end, encodes
  Ogg Vorbis, uploads both via AnkiConnect (`Image` / `SentAudio` fields).
- `vn-vad.py` — silero-VAD v5 (ONNX) speech boundary detection.
- `vn-record.sh` / `vn-screenshot.sh` — older replay-based scripts (press
  right-arrow to replay, record 8s). Still work for VNs with a replay key.

## Setup

```sh
python3 -m venv ~/.local/share/vn-mine/venv
~/.local/share/vn-mine/venv/bin/pip install onnxruntime numpy
curl -sL -o ~/.local/share/vn-mine/silero_vad.onnx \
  https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx

cp vn-buffer.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now vn-buffer
```

Bind `vn-capture.sh` to a KDE shortcut. Requires: ffmpeg, wl-clipboard,
pactl (pipewire-pulse), spectacle, curl, jq; Textractor copying hooked lines
to the clipboard; Anki with AnkiConnect on :8765.

## Usage

Read the line → look things up → create the Anki card → **press the hotkey
before advancing** (a new hooked line becomes "the last line"). Click back to
the VN window first so the screenshot captures it.

- The daemon binds the default sink at startup — `systemctl --user restart
  vn-buffer` after switching audio outputs.
- `VN_DRY=1 ./vn-capture.sh` — build clip + screenshot, skip Anki, keep files.
- `VN_MAX_LEN` (default 20) — max seconds considered after the line appears.
- `VN_VAD_THRESHOLD` (default 0.5) — raise if BGM vocals leak in, lower if
  quiet lines get cut.
