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
  restarts. Also inserts each line into the shared knowledge DB
  (`~/.local/share/jp-tools/knowledge.db`, override with
  `JP_TOOLS_KNOWLEDGE_DB_PATH`) so reading time/chars are tracked
  automatically — best-effort, never blocks mining; disable with
  `JP_TOOLS_STATS_DISABLE=1`. read-stats' own DB is attached alongside for the
  `current_work` setting, which is the title stamped on each line.

  **Restarting the logger with Textractor attached is safe**, as long as it
  goes through SIGTERM: `run()` sends a proper close frame before exiting, and
  the capture-pause path reuses the same `ws.close()`. Verified 2026-07-25 —
  `systemctl --user restart vn-buffer` plus a pause/resume cycle, Textractor
  alive throughout. What the WS plugin cannot survive is an **abortive**
  disconnect (`kill -9`, or a crash that skips the close frame), so don't hard-
  kill it and don't drop the signal handler.
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

read-stats' `#read` view shows the same line feed and runs this script itself
on every card add, so there is no hotkey to press there.

Set `VN_WINDOW` (or the current work's window in read-stats) and the "click back
first" step disappears: the screenshot then targets the VN window directly
instead of whatever happens to be focused.

The voiceline anchor is the moment Textractor hooks the line (a re-hook of the
line still on screen — a double-fire — does not move it). The audio must still
be in the ring, so press the hotkey within ~5 minutes of the line playing. If
no speech is detected in the window, the note gets the screenshot only (no
audio) and a warning notification says so — usual causes are an unvoiced line,
a stale anchor, or audio playing on a different output than the one the daemon
recorded (restart vn-buffer after switching outputs).

The window also **ends at the next hooked line**, when one has been hooked by
the time the capture runs. `VN_MAX_LEN` alone bounds it at ten seconds, which
is ten seconds of reading on — so a line mined and advanced past would take the
*following* line's voiceline, and an unvoiced line would take it entirely,
since its own span holds no speech to prefer. Nothing from the next line
onwards can be this line's voice, so nothing from there on is searched. If that
leaves less than `VN_MIN_LEN` of audio after the line, the capture is
screenshot-only: the reader advanced at once, so either the line was not voiced
or its voice was not waited for. Pressing the hotkey while the line is still on
screen is unaffected — there is no next line yet to bound anything.

The trimmed clip is then checked twice more before it is attached, because
silero is stateful and a loud transient partway through the window can cross
the threshold on state the preceding audio warmed up: the clip has to peak
above `VN_MIN_PEAK_DB`, and it has to still read as speech when the model
scores it on its own from a cold state. Either failing makes the capture
screenshot-only. Measured over a day of mining, voiced clips score ~1.00
standalone; the sound effect that prompted the check scored 0.35.

- The daemon binds the default sink at startup — `systemctl --user restart
  vn-buffer` after switching audio outputs. Safe to do with Textractor open —
  the logger closes the WebSocket cleanly on SIGTERM. A hard kill bypasses
  that, and an abortive disconnect is what crashes the WS plugin.
- `VN_WS_URL` (default `ws://localhost:6677`) — Textractor WebSocket server.
- `VN_DRY=1 ./vn-capture.sh` — build clip + screenshot, skip Anki, keep files.
- `VN_JSON=1 ./vn-capture.sh` — print a result object
  (`{ok, note_id, duration, note, line}` or `{ok: false, error}`) on stdout and
  suppress every `notify-send`. This is how read-stats' `#read` view runs the
  script — the result goes back to the browser that mined, which may not be on
  this desktop; see `read-stats/README.md`.
- `VN_WINDOW` — substring of the VN window's title (e.g. `素晴らしき日々`).
  When set, the screenshot is taken of *that window by id* rather than of
  whatever has focus, so it stays correct no matter what was focused when the
  capture fired. Needed to mine from read-stats' `#read` page in a browser on
  this machine — the browser is focused at that moment, so the default would
  capture the browser. Requires `xdotool` and ImageMagick's `import`; Wine/Proton
  windows are XWayland, so this works under a Wayland session even though
  `xdotool getactivewindow` does not. Unset, unmatched, or missing tools fall
  back to the active window and say so in the result. **Unset, it falls back to
  the current work's `vn_window` in read-stats** (read straight from the stats
  DB — the column on the currently-selected work, with the legacy global
  `vn_window` setting as a last resort), so the hotkey and the `#read` mine
  button share one place to configure this. Set it from the dashboard's
  *Currently reading* card (under **edit**), which offers a picker of open
  windows. Because it's tied to the work, switching VNs switches the capture
  target with it — no second place to keep in sync.
- `VN_MAX_LEN` (default 10) — max seconds considered after the line appears.
  No voiceline runs that long; a wider window only gives a false positive more
  room to be found in. The next hooked line cuts the window short of this
  whenever it lands first.
- `VN_MIN_LEN` (default 0.6) — once the next line has bounded the window, the
  least audio after the line still worth running VAD over. Below it the capture
  is screenshot-only rather than a guess at a voiceline that had no time to
  play.
- `VN_VAD_THRESHOLD` (default 0.5) — raise if BGM vocals leak in, lower if
  quiet lines get cut.
- `VN_VAD_MIN_SPEECH` (default 0.5) — ignore detected speech shorter than this.
  Above the length of a sound effect, below the length of even a one-word line.
- `VN_MIN_PEAK_DB` (default -25) — reject a clip whose peak never reaches this.
  A silence floor, not a voice test: real voicelines peak around -6 to -16
  dBFS, but so do sound effects.
- `VN_WHISPER_URL` (default `http://localhost:8100`) — whisper-service used
  for the sentence trim. If unreachable, clips are attached untrimmed.
