# capture — audio and screenshots for a mined card

Single-hotkey visual novel sentence mining: attach the last voiceline's audio
and a screenshot of the game to the most recently added note of the configured
note type (`KOTODEX_ANKI_MODEL`, Lapis by default).

Works without any in-game voice replay: a daemon keeps the last 300s of
desktop audio in a tmpfs ring buffer and timestamps every Japanese line
Textractor hooks (read from its WebSocket server — the same feed the
texthooker-ui uses) — the hook moment marks the voiceline start, and
silero-VAD finds where the speech ends.

## The path

```
                        ┌───────────────────────────────────────────┐
  desktop audio         │  kotodex-capture  (daemon, always on)     │
  (sink monitor)  ────▶│  ffmpeg -f segment                        │
                        │  5s WAV × 60 in tmpfs, wrapping ring      │
                        │  ≈ the last 300s always on disk           │
                        └───────────────────────┬───────────────────┘
                                               │
  Textractor ──▶ POST /api/lines ──▶ ledger    │  lines.log
       (hooked line + timestamp)               │  ts ⇥ text
                                               │       │
                         ┌──────────────────────▼───────▼──────────┐
  hotkey / card add ───▶│           vn-capture.sh                 │
                         └─────────────────────┬───────────────────┘
                                               │
       ┌───────────────────────────────────────┴───────────────────┐
       │ 1. ANCHOR                                                 │
       │    the newest line at the press (VN_ANCHOR_TS), not now   │
       │    window = [line_ts , next_line_ts or +VN_MAX_LEN]       │
       └───────────────────────────────┬───────────────────────────┘
                                       │ cut the window out of the ring
                                       ▼
       ┌──────────────────────────────────────────────────────────┐
       │ 2. FIND SPEECH   vn-vad.py  (silero-VAD v5 ONNX)         │
       │    16k mono → first_start / last_end                     │
       │    then peak ≥ VN_MIN_PEAK_DB and a cold-state rescore   │
       └───────────────┬──────────────────────┬───────────────────┘
                       │ speech               │ no speech, or VAD failed
                       ▼                      ▼
       ┌────────────────────────────────┐ ┌───────────────────────┐
       │ 3. TRIM TO SENTENCE            │ │ screenshot only, or   │
       │    vn-trim.py — only when the  │ │ the untrimmed window  │
       │    line holds several          │ └────────────┬──────────┘
       │    sentences                   │              │
       │                                │              │
       │  whisper-service /transcribe   │              │
       │    ?words=true → word times    │              │
       │            │                   │              │
       │            ▼                   │              │
       │  difflib-align the mined       │              │
       │  sentence against the          │              │
       │  punctuation-stripped          │              │
       │  transcript                    │              │
       │            │                   │              │
       │   coverage ≥ 0.6 ?             │              │
       │    yes │        │ no           │              │
       │        │        ▼              │              │
       │        │  anchor on the mined  │              │
       │        │  word, expand to 。！？│              │
       │        │  or a ≥0.5s gap       │              │
       │        ▼        ▼              │              │
       │  snap the edges to a real VAD  │              │
       │  silence (±0.25s), pad         │              │
       │  0.30 pre / 0.25 post          │              │
       │            │                   │              │
       │   no confident match → "none", │              │
       │   keep the whole clip          │              │
       └────────────┬───────────────────┘              │
                    ▼                                  │
       ┌───────────────────────────────────────────────▼──────────┐
       │ 4. ENCODE AND ATTACH                                     │
       │    ffmpeg → Ogg Vorbis q3        screenshot → png        │
       │    AnkiConnect storeMediaFile → updateNoteFields         │
       └──────────────────────────────────────────────────────────┘
```

Every branch degrades downward, never sideways: no trim keeps the whole window,
no speech keeps the screenshot, no window falls back to whatever has focus.

## Components

- `kotodex-capture` — daemon: ffmpeg ring buffer (60 × 5s WAV segments from the
  default sink monitor) plus the Textractor source
  (`sources/textractor/vn-ws-logger.py`), both writing into
  `$XDG_RUNTIME_DIR/kotodex/`. It runs the source as well as the ring buffer
  because a capture needs the line's timestamp and the audio to have been
  taken by the same process — one daemon, one clock. Started and stopped by
  the Kotodex launcher;
  `kotodex-capture {run|stop|restart|status}` drives it by hand, and delegates
  to the systemd unit when there is one.
- `vn-capture.sh` — bind to a hotkey. Screenshots the VN's window, cuts
  audio from the last hooked line's timestamp to the VAD speech end, encodes
  Ogg Vorbis, uploads both via AnkiConnect into the note type's image and audio
  fields (`KOTODEX_ANKI_FIELD_IMAGE` / `_AUDIO`, the same map the Rust side
  reads, so one note type is described in one place).
  Which window that is comes from kotodex-server (`GET /api/vn/window`), not from
  the database — the hotkey and the mine button have to aim at the same one, and
  two implementations of that rule means the one you forget captures the last
  game. No answer falls back to whatever has focus.
- `vn-vad.py` — silero-VAD v5 (ONNX) speech boundary detection.
- `vn-trim.py` — trims the clip to the mined sentence. A hooked line can hold
  several sentences while Yomitan mines one; this transcribes the clip with
  word timestamps (whisper-service `?words=true`), difflib-aligns the note's
  sentence field against the transcript (tolerant of wrong-kanji ASR), and cuts
  at the matched span. Falls back to anchoring on the vocab field and expanding
  to punctuation/silence boundaries; on any failure the VAD-trimmed clip is
  kept unchanged. Needs whisper-service running on :8100.
Everything here is on one path: the daemon keeps the ring buffer, the source
timestamps the lines beside it, and a capture reads what they left. This is
Linux-only and optional — the ledger, the dashboard and the reader all work
without it, and a card made without it simply has no audio or screenshot.

## Reading over the game

The line and its dictionary drawn over the VN is `kotodex-server/overlay/`, and the
Qt shell that puts it above a fullscreen window is `layer-overlay/`. Neither
shares any code with the capture pipeline here: the page is a kotodex-server client
and the shell takes a URL.

```sh
kotodex-server/overlay/vn-overlay.sh          # start, or restart what is up
```

What the two halves do share is the ledger. `vn-ws-logger.py` posts the lines
the overlay reads, and every card the overlay makes fires `vn-capture.sh` for
its audio and screenshot — so the overlay is only as live as this daemon is.
The badge in its corner reports exactly that.

## Setup

```sh
python3 -m venv ~/.local/share/kotodex/venv
~/.local/share/kotodex/venv/bin/pip install -r capture/requirements.txt
curl -sL -o ~/.local/share/kotodex/silero_vad.onnx \
  https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx

ln -sf "$PWD/kotodex-capture" ~/.local/bin/kotodex-capture
```

Launching Kotodex starts the daemon and quitting stops it, which is all most
setups need — reading is the only thing the ring buffer is for.

The systemd unit is for keeping it up independently of the launcher:

```sh
cp kotodex-capture.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now kotodex-capture
```

Only one of the two should own it. A unit that is running is *adopted* by the
launcher — never started twice, and never stopped on the way out — so with the
unit enabled, quitting Kotodex leaves capture running and picking up new code
means `kotodex restart` rather than relaunching.

Bind `vn-capture.sh` to a desktop shortcut. Requires: ffmpeg, pactl
(pipewire-pulse), curl, jq, and one screenshot tool — grim, spectacle,
gnome-screenshot or ImageMagick's `import`, whichever the desktop has;
Textractor with a WebSocket server extension on `ws://localhost:6677` (the feed
the texthooker-ui reads); Anki with AnkiConnect on :8765.

## Usage

Read the line → look things up → create the Anki card → **press the hotkey
before advancing** (a new hooked line becomes "the last line"). Click back to
the VN window first so the screenshot captures it.

kotodex-server's `#read` view shows the same line feed and runs this script itself
on every card add, so there is no hotkey to press there.

Set `VN_WINDOW` (or the current work's window in kotodex-server) and the "click back
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
  kotodex-capture` after switching audio outputs. Safe with Textractor open.
- `VN_WS_URL` (default `ws://localhost:6677`) — Textractor WebSocket server.
- `VN_DRY=1 ./vn-capture.sh` — build clip + screenshot, skip Anki, keep files.
- `VN_JSON=1 ./vn-capture.sh` — print a result object
  (`{ok, note_id, duration, note, line}` or `{ok: false, error}`) on stdout and
  suppress every `notify-send`. This is how kotodex-server runs the script: the
  result goes back to the browser that mined, which may not be on this desktop.
- `VN_WINDOW` — substring of the VN window's title. Set, the screenshot targets
  *that window by id* rather than whatever has focus, which is what makes mining
  from `#read` in a browser work — the browser is what's focused at that moment.
  Requires `xdotool` and ImageMagick's `import`; Wine/Proton windows are
  XWayland, so this works under Wayland even though `xdotool getactivewindow`
  does not. Unset, unmatched or missing tools fall back to the active window and
  say so in the result.

  **Unset, it falls back to the current work's `vn_window` in kotodex-server**, so
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
