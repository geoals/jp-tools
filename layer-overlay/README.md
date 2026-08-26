# layer-overlay

A web page drawn **over** everything else, fullscreen windows included, and
clickable only where the page says it has drawn something.

Nothing here knows what the page is for. There is no Japanese in it, no
dictionary, no backend — give it a URL and it shows that page over everything.
`read-stats/overlay/` is the one caller today.

```python
import layer_overlay

layer_overlay.run("http://localhost:3200/overlay/overlay.html",
                  scope="my-overlay", storage="~/.local/share/my-overlay")
```

## Two backends

Which mechanism puts the surface above a fullscreen window is a property of the
compositor, not of this program, so it is chosen at startup and the page cannot
tell which it got:

- **layer-shell** — `zwlr_layer_shell_v1` through `org.kde.layershell`. A
  surface on the overlay layer is above everything by protocol. KDE and wlroots
  offer it; GNOME does not.
- **x11** — `_NET_WM_STATE_ABOVE` on an XWayland window, with the input region
  set through XShape. Works wherever XWayland does, GNOME included.

`backend.py` picks one and prints which and why; `LAYER_OVERLAY_BACKEND` forces
one. The choice has to be made before `QGuiApplication` exists, because it
decides the Qt platform plugin.

- `layer_overlay.py` — the shell: the surface, the input region, the tracked
  window's geometry, and `run()`.
- `backend.py` — which backend, and the environment Qt reads at construction.
- `Overlay.qml` / `OverlayX11.qml` — the surface itself, one per backend, each
  holding one `WebEngineView`.
- `xshape.py` — the X11 input shape, which is not what `QWindow.setMask` sets.
- `xwatch.py` — where the tracked window is, told by X rather than polled.
- `runner.sh` — sourced by a caller's launcher for
  `start`/`stop`/`restart`/`ensure`/`status`, detached from the shell that
  started it.

Needs PySide6 with Qt WebEngine. The layer-shell backend needs `layer-shell-qt`
as well, and all of them as **system packages** — a pip PySide6 carries its own
Qt, which does not read the distribution's QML path, so a system
`org.kde.layershell` beside it is not loadable. `backend.py` checks that and
picks x11 instead, which is why a venv build still runs.

## The input region is the design

The page reports the boxes it has drawn, and the shell makes them the surface's
input region — `wl_surface.set_input_region` under layer-shell, an XShape
`ShapeInput` under X11: a click on one of them reaches the page, a click anywhere
else reaches the window underneath. No mode to switch.

That leaves the page nothing to dismiss things with, since a click outside never
arrives — it has to close on its own terms.

`SIGUSR1` makes the *whole* surface take input, for selecting text rather than
clicking through. `SIGUSR2` is passed to the page as `shell.userToggled` and
means whatever the page decides. Both go by the caller's script name:

```sh
pkill -USR1 -f my-overlay.py
```

## What the page has to do

`qwebchannel.js` is injected before the page runs, so the page carries no copy
of it. Connect to the object registered as `shell`:

| call | does |
|---|---|
| `shell.setHits([x, y, w, h, ...])` | what takes clicks, flat |
| `shell.setWindowName(name)` | track this window's rectangle, by title substring |
| `shell.geometry(x, y, w, h)` | where it is now, zeros when it cannot be found |
| `shell.userToggled()` | SIGUSR2 reached the page |
| `shell.quit()` | close; `run()` returns `QUIT_REQUESTED` rather than 0 |

Push the hits **the instant the layout changes**. Any lag, and any gap between
two boxes that ought to touch, is a click aimed at the page landing on the
window underneath instead.

The rectangle arrives in the page's own coordinates: CSS pixels, and relative to
the surface's origin rather than the screen's. Both conversions are identity
under layer-shell and neither is under X11, where a window manager shrinks the
surface to the work area.

Window tracking is X — `xwatch.py` selects for the events that mean a window
moved, so the rectangle arrives with the window rather than an interval behind
it, and falls back to polling `xdotool` where no X connection can be opened.
Either way it finds XWayland windows, Wine and Proton games among them, and not
Wayland-native ones, which report zeros.
