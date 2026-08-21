# layer-overlay

A web page drawn **over** everything else, fullscreen windows included, and
clickable only where the page says it has drawn something.

Nothing here knows what the page is for. There is no Japanese in it, no
dictionary, no backend — give it a URL and it shows that page as a
`zwlr_layer_shell_v1` overlay surface. `read-stats/overlay/` is the one caller
today.

```python
import layer_overlay

layer_overlay.run("http://localhost:3200/overlay/overlay.html",
                  scope="my-overlay", storage="~/.local/share/my-overlay")
```

- `layer_overlay.py` — the shell: the surface, the input region, the tracked
  window's geometry, and `run()`.
- `Overlay.qml` — the surface itself, holding one `WebEngineView`.
- `runner.sh` — sourced by a caller's launcher for `start`/`stop`/`restart`/
  `status`, detached from the shell that started it.

Needs PySide6, qt6-webengine and layer-shell-qt as **system packages** — a venv
build of PySide6 carries no `org.kde.layershell`.

## The input region is the design

The page reports the boxes it has drawn, and Qt hands them to
`wl_surface.set_input_region`: a click on one of them reaches the page, a click
anywhere else reaches the window underneath. No mode to switch.

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

Push the hits **the instant the layout changes**. Any lag, and any gap between
two boxes that ought to touch, is a click aimed at the page landing on the
window underneath instead.

Window tracking is `xdotool`, so it finds XWayland windows — Wine and Proton
games among them — and not Wayland-native ones, which report zeros.
