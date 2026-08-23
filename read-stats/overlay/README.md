# read-stats/overlay — reading in fullscreen

The line and its dictionary drawn **over** the game, fullscreen included.

`#read` has to sit beside the VN, because Yomitan needs a browser window and a
browser window loses to a fullscreen one. KWin puts a `zwlr_layer_shell_v1`
overlay surface *above* fullscreen windows, so the line can sit on the game.

```sh
read-stats/overlay/vn-overlay.sh                  # start, or restart what is up
read-stats/overlay/vn-overlay.sh --mobile         # 1.75x, read off a phone
read-stats/overlay/vn-overlay.sh stop|status
```

Needs PySide6, qt6-webengine and layer-shell-qt — **system packages, not the
venv**, which is why `vn-overlay.sh` calls bare `python3`.

- `overlay.html` / `overlay.js` — the page. Vanilla JS, sharing only
  `web-shared/` with read-stats' own frontend.
- `vn-overlay.py` — which page, and the two health checks. Everything about
  *being* an overlay is `layer-overlay/`, which knows nothing about reading.
- `vn-overlay.sh` — start it from anywhere, including over ssh, and keep it
  alive when that shell goes. Starting stops whatever is already running, so
  there is only ever one.

**read-stats is the backend, and serves this directory at `/overlay/`.** The
page calls eight `/api` routes — the line stream, the dictionary, the ledger,
the card — and none of them can be answered anywhere else, since the dictionary
and the ledger are jp-core's. Loading it over `file://` would break every
relative `fetch`, and an absolute URL would need CORS for nothing. It is served
straight off disk, but the view only reads it at load, so an edit is picked up
by `vn-overlay.sh restart`.

Starting without read-stats warns and carries on; the strip fills in when
read-stats arrives, because `EventSource` reconnects on its own. Anki down
warns too — mining is what fails. Textractor is not checked here: the page
already reports it live in the corner, from the logger's heartbeat.

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
for selecting text rather than advancing. `SIGUSR2` toggles ghost mode, the
same thing the checkbox under ⚙ → Marks does. Both are `layer-overlay`'s; see its README for
the input region itself.
`vn-overlay.py` runs perfectly well by hand; the wrapper only handles being
started from somewhere without a desktop session attached.

Three actions on a word, and only one of them opens the popup:

| action              | what it does                        | lookup recorded |
| ------------------- | ----------------------------------- | --------------- |
| left click          | the definition                      | yes             |
| back (side button)  | toggle known ⇄ unknown              | no              |
| forward             | mine it                             | no              |
| wheel               | page the open popup's dictionaries  | no              |

Opening the popup *is* the lookup, so it is the only thing that counts as one.
Reaching a button through the popup meant judging a word already understood
recorded a lookup that never happened, which is why the side buttons carry those
two. Judging repaints the word; mining reports with a desktop notification and
nothing else.

The popup head carries the same three actions as small ✓ / ✗ / ＋ buttons, sized
and bordered like the frequency pills beside them — not every way of reading
this has side mouse buttons, and driving the PC's mouse from a phone as a
touchpad has none. Marking a word **known** there posts
`/api/reader/lookup/retract`, which deletes the row that opening the popup
recorded: the popup was opened to reach the button, not
to read the definition. Only known retracts — not knowing a word whose
definition is on screen is what a lookup *is*, and so is mining one. The client
hands back the id `define` returned, so a retraction can only ever undo the one
row that popup made. The side buttons still cost nothing at all and stay the
way to judge a word without asking what it means.

**The controls live behind one handle.** ☰ at the top left is the bar shut:
drag it to move the widget, click it to shut explain, hide the line and
settings, which sit in a row to its right. It starts open and inverts while it
is, and only the handle opens and shuts it — nothing closes it on its own, so a
button is never taken out from under the pointer reaching for it. Paused tints
the handle, since that is the one state worth seeing while the bar is shut.

**⚙ is three tabs**, because the questions are different. *Text* — size, line
height, spacing, weight, backdrop, shadow strength and spread, colour, font.
*Placement* — column width, phone size and the scale it uses. *Marks* — whether
a status is painted and which, how strongly, the common-word threshold, and
ghost mode. Pausing capture is under the tabs: it is the one action in the
panel rather than a setting.

The shadow is centred on the glyphs rather than dropped below them — what it is
for here is lifting the character off the artwork, not casting it in a
direction.

**The colour and the font are both in-page.** A layer surface has nowhere to
open a native window, so neither the browser's colour picker nor a `<select>`
would appear at all: the colour is hue, saturation and lightness sliders, and
the font list is every Japanese-capable family `fc-list` reports, from
`GET /api/reader/fonts`, each name drawn in its own face.

Most of it is stored in this browser, because it is about this screen — a phone
reading the same overlay wants its own. The two every reading surface has to
agree on, status marks and the common-word threshold, are read from and written
back to read-stats, so `#read` and the overlay cannot disagree about the same
word.

`--mobile` draws the overlay at 1.75x with the line on the bottom edge, for
reading the screen off a phone. ⚙ → Placement switches between
the two without restarting the shell — it reloads the page with the layout's
query parameters flipped, and the stream replays the newest line on reconnect.

**The line is placed against the game's window, not the screen.** read-stats
puts the current work's `vn_window` on the status event — the same column
`vn-capture.sh` screenshots by, so there is still one place to say which window
is the game — and the shell polls `xdotool` for its rectangle and pushes it over
the WebChannel. Everything in `overlay.html`'s `--text-*` is a fraction of that
rectangle, so moving or resizing the game carries the line with it and another
resolution needs no re-measuring. No rectangle — no name on the work, no
`xdotool`, a Wayland-native game — and the line falls back to sitting against
the screen, which is where it sat before any of this.

The `--text-*` defaults are measured off one VN. Another wants its own: drag the
line onto the game's own text to find the offset, which is stored as fractions
of the window, then set `--text-x`, `--text-y`, `--text-w` and `--text-size`
from where it landed. They are per install, not yet per work.

**Ghost mode** (⚙ → Marks, or `SIGUSR2`) draws the line invisibly over the
game's own text: the game does the typesetting and the overlay contributes only
the status tint per word, the underline on a common word, and somewhere to
click. It needs the line to sit on the game's text to the pixel, so it only
engages while the window rectangle is known, and it is only as good as the
calibration above — a font whose advance differs from the game's drifts along
the line until the tint sits between two words. The tints drop to a wash at that
weight, since here they are over glyphs rather than under them.

**♪ plays the word.** The Local Audio Server add-on beside Anki holds NHK,
新明解, Forvo and JPod recordings and ranks them, so the button plays the first
and names it on hover — the same recording Yomitan would put on a card.
read-stats proxies it (`/api/reader/audio`), because that server binds loopback
and sends no CORS headers, so neither this page nor a phone could reach it. A
word with no recording shows no button.

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

Debug through Python, not the log. `LAYER_OVERLAY_DEBUG=1` prints the input region
on every change.

- `VN_OVERLAY_URL` — page to show (default `overlay.html` beside it, over
  read-stats on :3200).
- `JP_TOOLS_ANKI_URL` (default `http://localhost:8765`) — checked at startup
  only; the card itself is added by read-stats.
- `VN_OVERLAY_HEIGHT` (default 300, 525 under `--mobile`) — strip height, px. The
  text is positioned against it, so changing it moves the line by the same
  amount.
- `VN_OVERLAY_BG` (default 0.82) — backdrop alpha. At 1 the game's own text is
  hidden, which is the only thing that makes the two agree: the VN's line
  breaks are inserted when it renders, so they are not in the hooked text and
  cannot be reproduced.
- `VN_OVERLAY_FONT` (default `DNP Shuei Mincho Pr6`, set by `vn-overlay.sh`;
  unset it falls back to the page's `Noto Sans CJK JP`) — the font for the line
  only.
  Any family name `fc-list :lang=ja family` prints. The popup keeps the default:
  a dictionary and the text being tried are hard to judge in the same face.

