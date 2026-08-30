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
- `winfocus.py` / `kdefocus.py` — whether the tracked window is the one in use,
  reported to the page as `inFront`. Windows asks, KDE is told.
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

## Which window is in use, and staying above it

The surface is topmost, so a browser or an editor brought to the
front is drawn *under* it — right while the tracked window is being read and
wrong the moment anything else is. `winfocus.py` answers which of the two it is:
the foreground window belonging to the tracked window's process, or to this one,
or no window being tracked at all. The answer reaches the page as `inFront`, and
what to do with it is the page's — it can drop what it lays over the tracked
window and keep its own controls, which is what nothing being hidden here buys.

Hiding the whole surface is the obvious alternative and it costs the page: with
nothing on screen there is no way to reach the overlay, so a reader who started
it with another window in front sees nothing until they focus the game.

This process counting as the tracked one is the whole of why the answer is
stable. The same rule written against focus alone oscillates: the surface takes
focus, which reads as the game no longer being in front.

A window coming to the front may bring the topmost band with it — a game that
asserts topmost on focus, an upscaler that starts scaling into a topmost window
of its own — so every change is also a raise. One raise is a race against
whoever else is raising on the same event, so `wininput.raise_topmost` keeps
re-asserting for `SETTLE_MS`, after which it holds.

`kdefocus.py` answers the same question on KDE, and only there. No Wayland
protocol carries it: `wlr-foreign-toplevel-management`,
`ext-foreign-toplevel-list` and `plasma-window-management` each would, and KWin
advertises none of them to an unprivileged client. So it asks KWin, through a
script loaded over D-Bus that reports `windowActivated` — which is also the only
way the question is answerable at all here. Under a Wayland session the game is
in XWayland and the surface is native, and neither display server can be asked
something that answers for both; KWin is the compositor for both, so it can.

Nothing is raised beside it — a layer surface is above by protocol — so that
half of `_windows_tick` has no counterpart. Two things do differ from Windows.
Hiding waits out `SETTLE_MS`, because a KDE session activates windows nobody
chose and the panel taking focus and handing it straight back would otherwise
blink the line. And the tracked window is *found* by caption and then followed
by pid: its own menus carry captions of their own — a Kirikiri right-click menu
arrives as `kcMenuWindow` — and each would read as leaving it if the caption
were the whole test. Windows gets that for free from
`GetWindowThreadProcessId`; here the pid arrives beside the caption that matched.

Every other desktop has no gate, `inFront` is never emitted, and the page draws
as though it were true.

The other way to write this — putting the surface one place above the tracked
window in the z-order rather than at the top of everything, so any window brought
to the front covers it — is not available on Windows. `SetWindowPos` refuses a
z-order neighbour belonging to a process at a higher integrity level, with
`ERROR_ACCESS_DENIED`, and a VN run as administrator with an upscaler beside it
is exactly that. What is left is whole-band moves, and the one that leaves the
topmost band places the surface *above* the window that was just focused rather
than under it.

## The cursor an upscaler fences in

Windows only, and only while one is scaling. Magpie keeps the cursor inside the
window it is scaling — that is how the game goes on receiving mouse input from
under a picture drawn somewhere else — and draws its own cursor at the matching
place in the picture. So the reader points at the picture while the cursor
Windows reports, and the press it will deliver, are back in the small original.
The two agree at one point and diverge by the scale everywhere else.

The consequence is that the part of the picture outside the game's own window
cannot be clicked at all, and that is most of where a line belongs: at 4/3 the
fence is the middle three quarters of the picture, and it is only there by
coincidence — the two rectangles are related by nothing.

So `winwatch.aim` answers where the reader is pointing, and `wininput` takes the
cursor there when that is something the page has drawn. The fence is released
and the cursor moved, which sticks: an upscaler that has lost the cursor does not
take it back. The move is repeated for `ESCORT_POLLS`, because the position it is
handed is translated once more on the way out and the first move overshoots, and
then not attempted again for `ESCORT_REST_POLLS` — nothing observed re-fences a
cursor that has been taken out, and a cursor shaking in place would be worse than
one that cannot reach the line. Never while a button is down: that is a drag, or
a click meant for the game.

## What the page has to do

`qwebchannel.js` is injected before the page runs, so the page carries no copy
of it. Connect to the object registered as `shell`:

| call | does |
|---|---|
| `shell.setHits([x, y, w, h, ...])` | what takes clicks, flat |
| `shell.setWindowName(name)` | track this window's rectangle, by title substring |
| `shell.geometry(x, y, w, h)` | where it is now, zeros when it cannot be found |
| `shell.inFront(bool)` | that window is the one in use, or something else is; Windows and KDE only |
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
