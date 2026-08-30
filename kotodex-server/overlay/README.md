# kotodex-server/overlay — reading in fullscreen

The line and its dictionary drawn **over** the game, fullscreen included.

`#read` has to sit beside the VN, because Yomitan needs a browser window and a
browser window loses to a fullscreen one. A surface that is above a fullscreen
window by protocol does not — that is `layer-overlay/`, which picks between a
`zwlr_layer_shell_v1` surface and an always-on-top XWayland window depending on
what the compositor offers.

```sh
kotodex-server/overlay/vn-overlay.sh                  # start, replacing a running one
kotodex-server/overlay/vn-overlay.sh --mobile         # 1.75x, read off a phone
kotodex-server/overlay/vn-overlay.sh ensure           # start only if none is running
kotodex-server/overlay/vn-overlay.sh restart|stop|status
```

Needs PySide6 with Qt WebEngine. The layer-shell backend needs
`layer-shell-qt` too, and all three as **system packages** — a pip PySide6
carries its own Qt and no `org.kde.layershell`. Where the distribution packages
none, `setup.sh` puts one in a venv and the X11 backend runs off that;
`vn-overlay.sh` resolves which interpreter that is through
`scripts/lib/platform.sh`.

- `overlay.html` / `overlay.js` — the page. Vanilla JS, sharing only
  `web-shared/` with kotodex-server's own frontend.
- `vn-overlay.py` — which page, and the two health checks. Everything about
  *being* an overlay is `layer-overlay/`, which knows nothing about reading.
- `vn-overlay.sh` — start it from anywhere, including over ssh, and keep it
  alive when that shell goes. `start` stops whatever is already running, so
  there is only ever one; `ensure` leaves a running one alone.

**kotodex-server is the backend, and serves this directory at `/overlay/`.** Every
route the page calls is one of kotodex-server's — the line stream, the dictionary,
the ledger, the card, the fonts, the audio — and none of them can be answered
anywhere else, since the dictionary and the ledger are jp-core's. Loading it over
`file://` would break every relative `fetch`, and an absolute URL would need CORS
for nothing. It is served straight off disk, but the view only reads it at load,
so an edit is picked up by `vn-overlay.sh restart`.

Starting without kotodex-server warns and carries on; the strip fills in when
kotodex-server arrives, because `EventSource` reconnects on its own. Anki down
warns too — mining is what fails. Textractor is not checked here: the page
already reports it live in the corner, from the logger's heartbeat.

**Clicks are the design.** The page reports the box it has drawn, and the shell
makes that the surface's input region — `wl_surface.set_input_region` under
layer-shell, an XShape input shape under X11: a click on the overlay looks a word
up, a click anywhere else reaches the VN and advances the line. No mode to
switch.
The report is **pushed over a WebChannel the instant the layout changes**, and
the popup opens flush against the top of the line box. Both are the same
requirement: any lag, and any gap between the two boxes, is a click that was
aimed at the popup landing on the VN — which advances the line and closes the
popup being aimed at. `qwebchannel.js` is injected from Qt's own resources, so
kotodex-server serves nothing for it.

Three actions on a word, and only one of them opens the popup:

| action              | what it does                        | lookup recorded |
| ------------------- | ----------------------------------- | --------------- |
| left click          | the definition                      | yes             |
| back (side button)  | toggle known ⇄ unknown              | no              |
| forward             | mine it                             | no              |
| wheel               | page the open popup's dictionaries  | no              |

Opening the popup *is* the lookup, so it is the only thing that counts as one.
Reaching a button through the popup makes judging a word already understood
record a lookup that never happened, which is why the side buttons carry those
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

**The controls live behind one handle.** The Kotodex mark at the top left is
the bar shut: drag it to move the widget, click it to hide the row to its
right — explain, hide the line, scrollback, pause capture, stats, settings, and
✕ quit. It starts open, and only the handle opens and shuts it — nothing closes
it on its own, so a button is never taken out from under the pointer reaching
for it. The three
panels that hang under the bar (scrollback, explain, settings) are alternatives
rather than a stack: opening one closes the others.

Stats opens the dashboard in the desktop's browser, never in this view — the
surface is the page, so navigating it away would take the overlay with it.

Pause is its own button in that row rather than a row inside the settings:
pausing is reached for mid-scene, and a control two clicks behind a cogwheel is
one that gets skipped. Its icon carries the state — two bars while capture runs,
a triangle while it is stopped.

**⚙ is three tabs**, because the questions are different. *Text* — theme, font
size, line height, spacing, weight, column width, backdrop, shadow strength and
spread, colour, font, and the switch to phone size. *Marks* — whether a status is
painted and which, how strongly, the common-word threshold, and ghost mode.
*Source* — Textractor's WebSocket or the clipboard, and the WebSocket's address.

The shadow is centred on the glyphs rather than dropped below them — what it is
for here is lifting the character off the artwork, not casting it in a
direction. Its strength is how many times it is drawn, not how opaque it is: a
single blurred shadow spreads what it has over its whole radius, so full opacity
is still faint and stacking is what darkens it.

**The colour and the font are both in-page.** A layer surface has nowhere to
open a native window, so neither the browser's colour picker nor a `<select>`
would appear at all: the colour is hue, saturation and lightness sliders in
one box, and the font list is every Japanese-capable family installed, from
`GET /api/reader/fonts` — the name in the panel's own face, since a display
font renders its own name too fine to pick out of a list, with あア亜 beside it
as the sample.

Most of it is stored in this browser, because it is about this screen — a phone
reading the same overlay wants its own. The two every reading surface has to
agree on, status marks and the common-word threshold, are read from and written
back to kotodex-server, so `#read` and the overlay cannot disagree about the same
word.

`--mobile` draws the overlay at 1.75x with the line on the bottom edge, for
reading the screen off a phone. ⚙ → Text → *Phone size* switches between the two
without restarting the shell — it reloads the page with the layout's query
parameters flipped, and the stream replays the newest line on reconnect.

**The line is placed against the game's window, not the screen.** kotodex-server
puts the current work's `vn_window` on the status event — the same column
`vn-capture.sh` screenshots by, so there is still one place to say which window
is the game — and the shell finds that window's rectangle and pushes it over the
WebChannel. It is told where the window is by X rather than asking again and
again (`layer-overlay/xwatch.py`), and falls back to polling `xdotool` where no X
connection can be opened. Everything in `overlay.html`'s `--text-*` is a fraction
of that rectangle, so moving or resizing the game carries the line with it and
another resolution needs no re-measuring. No rectangle — no name on the work, no
way to reach X, a Wayland-native game — and the line falls back to sitting
against the screen.

The `--text-*` defaults are measured off one VN. Another wants its own: drag the
line onto the game's own text to find the offset, which is stored as fractions
of the window, then set `--text-x`, `--text-y`, `--text-w` and `--text-size`
from where it landed. They are per install, not yet per work.

**Which is also why the line takes itself off screen while another window is in
front** (`shell.inFront`, Windows only). It is placed against the game's text, so
over a browser or an editor it is over that window's text instead. The controls
stay: they are how the overlay is reached, and the whole surface going away is
what leaves a reader who started it from behind another window with nothing to
click. The button in the bar is the same thing said by hand, and either reason
keeps the line off.

**Ghost mode** (⚙ → Marks) draws the line invisibly over the
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
kotodex-server proxies it (`/api/reader/audio`), because that server binds loopback
and sends no CORS headers, so neither this page nor a phone could reach it. A
word with no recording shows no button.

The popup carries a **mined** badge when the word is already a card, and
clicking it opens that card in Anki. The check is Anki's own duplicate check,
asked after the definition is drawn so a shut or slow Anki cannot hold it up,
and a mine made while the popup is open raises the badge from the id the add
returns — no reopening.

The card is built by kotodex-server (`routes/reader/mine.rs`) and handed to
`services::card::add_note`, which is where Yomitan's own add arrives too, so it
is enriched and captured identically. Every field name comes from
`jp_mine_core::config::AnkiConfig`, so the card fits whichever note type is
configured and nothing here spells one. The definition field is written with
Yomitan's own per-dictionary wrapper divs, since the note type styles
`.dict-<name>-body` rather than the glossary inside it, and carries Sankoku and
Jitendex only — the two that note type has rules for. The word's own recording
comes from the same Local Audio Server the ♪ button plays, which is where
Yomitan's audio sources point, so both surfaces attach the same file.

Yomitan does not run here, so alt-tab to `#read` when the tokenizer picks the
wrong boundary.

Three things Qt will not survive: **calling a PySide slot from inside a
`runJavaScript` callback segfaults** (a WebChannel slot is a different path and
is fine); **QML's `console.log` reaches nothing here**, so that segfault reads as
a timer that never fired; and
**`WebEngineScript` cannot be declared in QML** (it is a value type), so the
injected script is built in Python. Also: `WebEngineView.webChannel` wants a
`QQmlWebChannel`, which PySide does not expose, so the channel is declared in
QML and the shell object registered into it from there.

Debug through Python, not the log. `LAYER_OVERLAY_DEBUG=1` prints the input region
on every change.

- `VN_OVERLAY_URL` — page to show (default `overlay.html` beside it, over
  kotodex-server on :3200).
- `KOTODEX_ANKI_URL` (default `http://127.0.0.1:8765`) — checked at startup
  only; the card itself is added by kotodex-server.
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
  Any family name `GET /api/reader/fonts` lists. The popup keeps the default:
  a dictionary and the text being tried are hard to judge in the same face.

