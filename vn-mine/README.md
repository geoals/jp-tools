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

  **Restarting the logger with Textractor attached is safe** as long as it goes
  through SIGTERM: `run()` sends a close frame before exiting, and the
  capture-pause path reuses the same `ws.close()`. What the WS plugin cannot
  survive is an **abortive** disconnect (`kill -9`, or a crash that skips the
  close frame) — so don't hard-kill it, and don't drop the signal handler.
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
- `vn-calibrate.py` — measurement tool, not part of a capture. Scores every
  line in the ring's last ~5 minutes against the speech VAD finds, and reports
  how closely a voiceline's onset tracks the hook and how well the line's mora
  count predicts its duration. Those two spreads are what a stricter capture
  rule would have to be built on; run it **during reading** — idle on a menu
  gives a loud ring and no speech.

  Two traps it has already fallen into, so don't reintroduce them: letting
  every line search independently (unvoiced lines then claim the next line's
  voice and invent rates like 131 morae/s — the tell is duplicate `dur` on
  neighbouring rows), and selecting the sample by `|onset| < 1.0` before
  reporting that onsets fall within 1.0.
- `vn-record.sh` / `vn-screenshot.sh` — older replay-based scripts (press
  right-arrow to replay, record 8s). Still work for VNs with a replay key.
- `overlay/` — the line drawn **over** the game instead of beside it. See below.

## overlay/ — reading in fullscreen

`#read` has to sit beside the VN, because Yomitan needs a browser window and a
browser window loses to a fullscreen one. KWin puts a `zwlr_layer_shell_v1`
overlay surface *above* fullscreen windows, so the line can sit on the game.

```sh
python3 vn-mine/overlay/vn-overlay.py
```

Needs PySide6, qt6-webengine and layer-shell-qt. The page is read-stats'
`/static/overlay.html`, so CSS edits are a reload, not a restart.

- `vn-overlay.py` — the shell: a layer surface, and the input region.
- `Overlay.qml` — the surface itself, holding one `WebEngineView`.

**Clicks are the design.** The page reports the box it has drawn, and Qt hands
that to `wl_surface.set_input_region`: a click on the overlay looks a word up,
a click anywhere else reaches the VN and advances the line. No mode to switch.
The report is **pushed over a WebChannel the instant the layout changes**, and
the popup opens flush against the top of the line box. Both are the same
requirement: any lag, and any gap between the two boxes, is a click that was
aimed at the popup landing on the VN — which advances the line and closes the
popup being aimed at. `qwebchannel.js` is injected from Qt's own resources, so
read-stats serves nothing for it.
`SIGUSR1` (`pkill -USR1 -f vn-overlay.py`) makes the whole surface take input,
for selecting text rather than advancing.

Three actions on a word, and only one of them opens the popup:

| action              | what it does                        | lookup recorded |
| ------------------- | ----------------------------------- | --------------- |
| left click          | the definition                      | yes             |
| back (side button)  | toggle known ⇄ unknown              | no              |
| forward             | mine it                             | no              |
| wheel               | page the open popup's dictionaries  | no              |

Opening the popup *is* the lookup, so it is the only thing that counts as one.
Reaching a button through the popup meant judging a word already understood
recorded a lookup that never happened, which is why the buttons are gone from
it. Judging repaints the word; mining reports with the chime and nothing else.

The popup carries a **mined** badge when the word is already a card, and
clicking it opens that card in Anki. The check is Anki's own duplicate check,
asked after the definition is drawn so a shut or slow Anki cannot hold it up,
and a mine made while the popup is open raises the badge from the id the add
returns — no reopening.

The card is built by read-stats and added through the AnkiConnect proxy Yomitan
uses, so it is enriched and captured identically. `VocabDefFull` is written with
Yomitan's own per-dictionary wrapper divs, since the note type styles
`.dict-<name>-body` rather than the glossary inside it, and carries Sankoku and
Jitendex only — the two that note type has rules for. `VocabAudio` is the one
field it cannot fill — Yomitan fetches that from its own audio sources.

Yomitan does not run here, so alt-tab to `#read` when the tokenizer picks the
wrong boundary.

Three things Qt will not survive, all found the hard way: **calling a PySide
slot from inside a `runJavaScript` callback segfaults** (a WebChannel slot is a
different path and is fine); **QML's `console.log` reaches nothing here** —
which is why that crash first looked like a timer failing to fire; and
**`WebEngineScript` cannot be declared in QML** (it is a value type), so the
injected script is built in Python. Also: `WebEngineView.webChannel` wants a
`QQmlWebChannel`, which PySide does not expose, so the channel is declared in
QML and the shell object registered into it from there.

Debug through Python, not the log. `VN_OVERLAY_DEBUG=1` prints the input region
on every change.

- `VN_OVERLAY_URL` — page to show (default read-stats' overlay page).
- `VN_OVERLAY_HEIGHT` (default 300) — strip height, px. The text is positioned
  against it, so changing it moves the line by the same amount.
- `VN_OVERLAY_BG` (default 0.75) — backdrop alpha. At 1 the game's own text is
  hidden, which is the only thing that makes the two agree: the VN's line
  breaks are inserted when it renders, so they are not in the hooked text and
  cannot be reproduced.
- `VN_OVERLAY_FONT` (default `Noto Sans CJK JP`) — the font for the line only.
  Any family name `fc-list :lang=ja family` prints. The popup keeps the default:
  a dictionary and the text being tried are hard to judge in the same face.

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
