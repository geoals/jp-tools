# layer-overlay

A web page drawn **over** everything else, fullscreen windows included, and
clickable only where the page says it has drawn something.

Nothing here knows what the page is for. There is no Japanese in it, no
dictionary, no backend — give it a URL and it shows that page over everything.
`kotodex-server/overlay/` is the one caller today.

```python
import layer_overlay

layer_overlay.run("http://localhost:3200/overlay/overlay.html",
                  scope="my-overlay", storage="~/.local/share/my-overlay")
```

## Three backends

Which mechanism puts the surface above a fullscreen window is a property of the
display server, not of this program, so it is chosen at startup and the page
cannot tell which it got:

- **layer-shell** — `zwlr_layer_shell_v1` through `org.kde.layershell`. A
  surface on the overlay layer is above everything by protocol. KDE and wlroots
  offer it; GNOME does not.
- **x11** — `_NET_WM_STATE_ABOVE` on an XWayland window, with the input region
  set through XShape. Works wherever XWayland does, GNOME included.
- **windows** — a layered topmost window. Windows has no input region, so the
  region is emulated: `WS_EX_TRANSPARENT` is toggled as the cursor crosses into
  what the page has drawn.

`backend.py` picks one and prints which and why; `LAYER_OVERLAY_BACKEND` forces
one. The choice has to be made before `QGuiApplication` exists, because it
decides the Qt platform plugin.

- `layer_overlay.py` — the shell: the surface, the input region, the tracked
  window's geometry, and `run()`.
- `backend.py` — which backend, and the environment Qt reads at construction.
- `Overlay.qml` — the layer surface. `OverlayWindow.qml` — the same surface as
  an ordinary always-on-top window, which is both the x11 and the windows
  backend. Each holds one `WebEngineView`.
- `xshape.py` / `wininput.py` — what takes clicks. Two implementations of one
  interface, because neither platform's is `QWindow.setMask`.
- `xwatch.py` / `winwatch.py` — where the tracked window is, told by the
  display server rather than polled.
- `runner.sh` — sourced by a caller's launcher for
  `start`/`stop`/`restart`/`ensure`/`status`, detached from the shell that
  started it. Linux only.

Needs PySide6 with Qt WebEngine. The layer-shell backend needs `layer-shell-qt`
as well, and all of them as **system packages** — a pip PySide6 carries its own
Qt, which does not read the distribution's QML path, so a system
`org.kde.layershell` beside it is not loadable. `backend.py` checks that and
picks x11 instead, which is why a venv build still runs. On Windows there is no
QML module to load beside Qt and `pip install PySide6` is the right way to get it.

## The input region is the design

The page reports the boxes it has drawn, and the shell makes them the surface's
input region — `wl_surface.set_input_region` under layer-shell, an XShape
`ShapeInput` under X11: a click on one of them reaches the page, a click anywhere
else reaches the window underneath. No mode to switch.

Windows has no such thing, and `SetWindowRgn` is the near miss to avoid — it
clips painting as well as input, so the surface would be visible only where it is
clickable. `wininput.py` emulates the region instead, by watching the cursor and
toggling `WS_EX_TRANSPARENT` as it crosses a boundary. The one behavioural
difference: what takes clicks follows the *cursor* there, not the click, so a
click delivered somewhere the cursor has not been arrives before the boundary is
noticed. No mouse can do that, since a click is preceded by the move that got
there.

That leaves the page nothing to dismiss things with, since a click outside never
arrives — it has to close on its own terms.

`SIGUSR1` makes the *whole* surface take input, for selecting text rather than
clicking through. `SIGUSR2` is passed to the page as `shell.userToggled` and
means whatever the page decides. Both go by the caller's script name:

```sh
pkill -USR1 -f my-overlay.py
```

Windows has neither signal, so both toggles are the page's own there until
something registers a hotkey for them.

## What the page has to do

`qwebchannel.js` is injected before the page runs, so the page carries no copy
of it. Connect to the object registered as `shell`:

| call | does |
|---|---|
| `shell.setHits([x, y, w, h, ...])` | what takes clicks, flat |
| `shell.setWindowName(name)` | track this window's rectangle, by title substring |
| `shell.geometry(x, y, w, h)` | where it is now, zeros when it cannot be found |
| `shell.userToggled()` | SIGUSR2 reached the page |
| `shell.openUrl(url)` | open an `http`/`https` link in the desktop's browser |
| `shell.quit()` | close; `run()` returns `QUIT_REQUESTED` rather than 0 |

Push the hits **the instant the layout changes**. Any lag, and any gap between
two boxes that ought to touch, is a click aimed at the page landing on the
window underneath instead.

The rectangle arrives in the page's own coordinates: CSS pixels, and relative to
the surface's origin rather than the screen's. Both conversions are identity
under layer-shell and neither is under X11, where a window manager shrinks the
surface to the work area.

Window tracking is the display server's — `xwatch.py` selects for the X events
that mean a window moved, `winwatch.py` takes a `SetWinEventHook` on the tracked
window's thread, so either way the rectangle arrives with the window rather than
an interval behind it. X falls back to polling `xdotool` where no connection can
be opened. It finds XWayland windows, Wine and Proton games among them, and not
Wayland-native ones, which report zeros.
