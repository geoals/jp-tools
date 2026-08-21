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

  **It creates neither database.** The schema is jp-core's migrations and
  read-stats'; this checks that the columns in `REQUIRED` are there and waits if
  they are not, retrying every 30s. Started before the migrations have run it
  logs to `lines.log` alone and picks the databases up when they appear, so a
  first boot loses nothing. Adding a column here means adding it to `REQUIRED`
  too, or the insert fails mid-session.

  **Restarting the logger with Textractor attached is safe** as long as it goes
  through SIGTERM: `run()` sends a close frame before exiting, and the
  capture-pause path reuses the same `ws.close()`. What the WS plugin cannot
  survive is an **abortive** disconnect (`kill -9`, or a crash that skips the
  close frame) — so don't hard-kill it, and don't drop the signal handler.
- `vn-capture.sh` — bind to a hotkey. Screenshots the VN's window, cuts
  audio from the last hooked line's timestamp to the VAD speech end, encodes
  Ogg Vorbis, uploads both via AnkiConnect (`Image` / `SentAudio` fields).
  Which window that is comes from read-stats (`GET /api/vn/window`), not from
  the database — the hotkey and the mine button have to aim at the same one, and
  two implementations of that rule means the one you forget captures the last
  game. No answer falls back to whatever has focus.
- `vn-vad.py` — silero-VAD v5 (ONNX) speech boundary detection.
- `vn-trim.py` — trims the clip to the mined sentence. A hooked line can hold
  several sentences while Yomitan mines one; this transcribes the clip with
  word timestamps (whisper-service `?words=true`), difflib-aligns the note's
  `SentKanji` against the transcript (tolerant of wrong-kanji ASR), and cuts
  at the matched span. Falls back to anchoring on `VocabKanji` and expanding
  to punctuation/silence boundaries; on any failure the VAD-trimmed clip is
  kept unchanged. Needs whisper-service running on :8100.
- `test_ws_logger.py` — the logger's tests. `python3 -m pytest vn-mine`.

Everything above is on one path: `vn-buffer.service` runs the ring buffer and
the logger, and a capture reads what they left. Nothing here is optional and
nothing is a spare copy.

## Reading over the game

The line and its dictionary drawn over the VN is `read-stats/overlay/`, and the
Qt shell that puts it above a fullscreen window is `layer-overlay/`. Neither
shares any code with the capture pipeline here: the page is a read-stats client
and the shell takes a URL.

```sh
read-stats/overlay/vn-overlay.sh          # start, or restart what is up
```

What the two halves do share is a database. `vn-ws-logger.py` writes the lines
the overlay reads, and every card the overlay makes fires `vn-capture.sh` for
its audio and screenshot — so the overlay is only as live as this daemon is.
The badge in its corner reports exactly that.

## Setup

```sh
python3 -m venv ~/.local/share/vn-mine/venv
~/.local/share/vn-mine/venv/bin/pip install -r vn-mine/requirements.txt
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

The voiceline anchor is the moment Textractor hooks the line; a re-hook of the
line still on screen does not move it. The audio must still be in the ring, so
press the hotkey within ~5 minutes. With no speech found in the window the note
gets the screenshot only and a warning says so — usually an unvoiced line, a
stale anchor, or audio on a different output than the daemon bound.

**The window ends at the next hooked line** whenever one has landed by the time
the capture runs. `VN_MAX_LEN` alone bounds it at ten seconds, which is ten
seconds of reading on, so a line mined and advanced past would take the
*following* line's voice. If that leaves less than `VN_MIN_LEN` of audio the
capture is screenshot-only: the reader advanced at once, so either the line was
unvoiced or its voice was not waited for. Mining while the line is still on
screen is unaffected — there is no next line yet to bound anything.

The clip is then checked twice more, because silero is stateful and a loud
transient can cross the threshold on state the preceding audio warmed up: it has
to peak above `VN_MIN_PEAK_DB`, **and** still read as speech when scored on its
own from a cold state. Either failing makes the capture screenshot-only. Voiced
clips score ~1.00 standalone; the sound effect that prompted the check scored
0.35.

- The daemon binds the default sink at startup — `systemctl --user restart
  vn-buffer` after switching audio outputs. Safe with Textractor open.
- `VN_WS_URL` (default `ws://localhost:6677`) — Textractor WebSocket server.
- `VN_DRY=1 ./vn-capture.sh` — build clip + screenshot, skip Anki, keep files.
- `VN_JSON=1 ./vn-capture.sh` — print a result object
  (`{ok, note_id, duration, note, line}` or `{ok: false, error}`) on stdout and
  suppress every `notify-send`. This is how read-stats runs the script: the
  result goes back to the browser that mined, which may not be on this desktop.
- `VN_WINDOW` — substring of the VN window's title. Set, the screenshot targets
  *that window by id* rather than whatever has focus, which is what makes mining
  from `#read` in a browser work — the browser is what's focused at that moment.
  Requires `xdotool` and ImageMagick's `import`; Wine/Proton windows are
  XWayland, so this works under Wayland even though `xdotool getactivewindow`
  does not. Unset, unmatched or missing tools fall back to the active window and
  say so in the result.

  **Unset, it falls back to the current work's `vn_window` in read-stats**, so
  the hotkey and the `#read` mine button share one place to configure it. Set it
  from the dashboard's *Currently reading* card (under **edit**), which offers a
  picker of open windows. Tied to the work, so switching VNs switches the capture
  target with it.
- `VN_MAX_LEN` (default 10) — max seconds considered after the line appears. No
  voiceline runs that long, and a wider window only gives a false positive more
  room to be found in. The next hooked line cuts it short whenever it lands
  first.
- `VN_MIN_LEN` (default 0.6) — the least audio after the line still worth running
  VAD over, once the next line has bounded the window.
- `VN_VAD_THRESHOLD` (default 0.5) — raise if BGM vocals leak in, lower if
  quiet lines get cut.

- `VN_VAD_MIN_SPEECH` (default 0.5) — ignore detected speech shorter than this:
  above a sound effect, below even a one-word line.
- `VN_MIN_PEAK_DB` (default -25) — reject a clip whose peak never reaches this.
  A silence floor, not a voice test: sound effects peak as loud as voicelines.
- `VN_WHISPER_URL` (default `http://localhost:8100`) — whisper-service used
  for the sentence trim. If unreachable, clips are attached untrimmed.
