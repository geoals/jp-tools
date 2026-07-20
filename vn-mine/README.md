# vn-mine

Single-hotkey visual novel sentence mining: attach the last voiceline's audio
and a screenshot of the active window to the most recently added
"Japanese sentences" Anki note.

Works without any in-game voice replay: a daemon keeps the last 300s of
desktop audio in a tmpfs ring buffer and timestamps every Japanese line
Textractor hooks (read from its WebSocket server — the same feed the
texthooker-ui uses) — the hook moment marks the voiceline start, and
silero-VAD finds where the speech ends.

## Components

- `vn-buffer.sh` — daemon: ffmpeg ring buffer (60 × 5s WAV segments from the
  default sink monitor) + `vn-ws-logger.py` hooked-line logger, both in
  `$XDG_RUNTIME_DIR/vn-mine/`. Run via the `vn-buffer.service` systemd user
  unit.
- `vn-ws-logger.py` — connects to the Textractor WebSocket server
  (`ws://localhost:6677`, override with `VN_WS_URL`) and appends each hooked
  Japanese line to `lines.log` with a timestamp. Auto-reconnects if Textractor
  restarts. Also inserts each line into the read-stats DB
  (`~/.local/share/jp-stats/stats.db`) so reading time/chars are tracked
  automatically — best-effort, never blocks mining; disable with
  `JP_TOOLS_STATS_DISABLE=1`.
- `vn-capture.sh` — bind to a hotkey. Screenshots the active window, cuts
  audio from the last hooked line's timestamp to the VAD speech end, encodes
  Ogg Vorbis, uploads both via AnkiConnect (`Image` / `SentAudio` fields).
- `vn-vad.py` — silero-VAD v5 (ONNX) speech boundary detection.
- `vn-trim.py` — trims the clip to the mined sentence. A hooked line can hold
  several sentences while Yomitan mines one; this transcribes the clip with
  word timestamps (whisper-service `?words=true`), difflib-aligns the note's
  `SentKanji` against the transcript (tolerant of wrong-kanji ASR), and cuts
  at the matched span. Falls back to anchoring on `VocabKanji` and expanding
  to punctuation/silence boundaries; on any failure the VAD-trimmed clip is
  kept unchanged. Needs whisper-service running on :8100.
- `vn-record.sh` / `vn-screenshot.sh` — older replay-based scripts (press
  right-arrow to replay, record 8s). Still work for VNs with a replay key.

## Setup

```sh
python3 -m venv ~/.local/share/vn-mine/venv
~/.local/share/vn-mine/venv/bin/pip install onnxruntime numpy websockets
curl -sL -o ~/.local/share/vn-mine/silero_vad.onnx \
  https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx

cp vn-buffer.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now vn-buffer
```

Bind `vn-capture.sh` to a KDE shortcut. Requires: ffmpeg, pactl
(pipewire-pulse), spectacle, curl, jq; Textractor with a WebSocket server
extension on `ws://localhost:6677` (the feed the texthooker-ui reads); Anki
with AnkiConnect on :8765.

## Usage

Read the line → look things up → create the Anki card → **press the hotkey
before advancing** (a new hooked line becomes "the last line"). Click back to
the VN window first so the screenshot captures it.

Reading from a phone instead? read-stats' `#read` view shows the same line feed
over the LAN and has a mine button that runs this script — the VN never loses
desktop focus that way, so the "click back first" step disappears.

The voiceline anchor is the moment Textractor hooks the line (a re-hook of the
line still on screen — a double-fire — does not move it). The audio must still
be in the ring, so press the hotkey within ~5 minutes of the line playing. If
no speech is detected in the window, the full window is attached and a warning
notification says so — usual causes are a stale anchor or audio playing on a
different output than the one the daemon recorded (restart vn-buffer after
switching outputs).

- The daemon binds the default sink at startup — `systemctl --user restart
  vn-buffer` after switching audio outputs. **Restart while Textractor is
  closed if possible**: the WS plugin can crash Textractor on an abortive
  client disconnect. The logger now sends a clean close frame on SIGTERM to
  mitigate this, but a hard kill still bypasses that.
- `VN_WS_URL` (default `ws://localhost:6677`) — Textractor WebSocket server.
- `VN_DRY=1 ./vn-capture.sh` — build clip + screenshot, skip Anki, keep files.
- `VN_JSON=1 ./vn-capture.sh` — print a result object
  (`{ok, note_id, duration, note, line}` or `{ok: false, error}`) on stdout and
  suppress every `notify-send`. This is how read-stats' `#read` view runs the
  script when you mine from your phone, where a desktop notification would go
  unseen; see `read-stats/README.md`.
- `VN_MAX_LEN` (default 20) — max seconds considered after the line appears.
- `VN_VAD_THRESHOLD` (default 0.5) — raise if BGM vocals leak in, lower if
  quiet lines get cut.
- `VN_WHISPER_URL` (default `http://localhost:8100`) — whisper-service used
  for the sentence trim. If unreachable, clips are attached untrimmed.
