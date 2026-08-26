# Which compositors the overlay works on

The overlay draws the line feed *above* a fullscreen game and takes clicks only
where it has drawn. Two mechanisms can do that, and which is available is a
property of the compositor, not of the app:

- **layer-shell** (`wlr-layer-shell`, via `org.kde.layershell`) — a surface on
  the overlay layer is above everything by protocol. This is what
  `layer-overlay/` uses today.
- **X11 always-on-top** (`_NET_WM_STATE_ABOVE` + XShape input region), running
  on XWayland under a Wayland session. Works wherever XWayland does.

Both are implemented. `layer-overlay/backend.py` picks one at startup and prints
which and why; `kotodex doctor` asks the same code rather than repeating its
rules.

## Results

| session | stays above fullscreen | click-through | xdotool geometry | verdict |
|---|---|---|---|---|
| KDE Wayland | yes (layer-shell) | yes | yes | supported — the development target |
| KDE Wayland, X11 backend forced | **no** | yes | yes | KWin puts an *active fullscreen* window above keep-above ones. Not a defect: KDE has layer-shell. |
| GNOME Wayland | yes (X11 backend) | untested | yes | supported via the X11 backend |
| Hyprland | expected yes (layer-shell) | — | — | untested; wlroots implements the protocol |
| GNOME Xorg | — | — | — | no such session; GNOME has dropped X11 |
| KDE Xorg | — | — | — | not installed here (`plasma-x11-session`) |

## GNOME Wayland, in detail

Tested with `scripts/spike/stack-test.py` — a frameless
`Qt.WindowStaysOnTopHint` window with `setMask` — over
`mpv --gpu-context=x11 --fs`.

It works: the probe stays above the fullscreen video, including after the video
is clicked. `xdotool search --name … getwindowgeometry` also works, since Wine
and Proton windows are XWayland, so window tracking is fine here.

**The Qt platform plugin has to be `xcb`.** On a native Wayland surface there is
no always-on-top protocol, so Qt accepts `WindowStaysOnTopHint` and silently
does nothing with it, and the overlay sinks behind any window that is clicked —
in windowed mode as much as fullscreen. PySide6 picks the Wayland plugin by
default whenever `qt6-wayland` is installed, so the X11 backend must set
`QT_QPA_PLATFORM=xcb` for itself rather than inherit the session's default.

## What the X11 backend had to do differently

Two things the layer-shell backend gets for free:

- **`QWindow.setMask` is the wrong call under X11.** Qt maps it onto the
  *bounding* shape, which clips what the window draws — so the surface would be
  visible only where it is clickable. `layer-overlay/xshape.py` sets
  `ShapeInput` instead, through libXext, and the bounding shape stays whole.
  Verified: with two hit rectangles reported, the input shape holds exactly
  those two and the bounding shape is the whole window.
- **The surface does not start at the screen origin.** A window manager shrinks
  it to the work area — 1920x1053+0+27 with a panel here — and asking for
  fullscreen or override-redirect does not get it back. Override-redirect also
  maps at the *bottom* of the stack. So the tracked window is translated into
  surface coordinates before the page sees it, which is identity under
  layer-shell.

Still to check on GNOME: that clicks land through the input region in a real
session, and that the surface stays above the game there as the spike measured.

## What this means for the release

The fullscreen overlay is not limited to layer-shell compositors. GNOME — the
largest single desktop — works through the X11 backend, which is what makes the
"90% of desktop Linux" claim defensible rather than something to narrow.

Backend selection at startup: layer-shell where the compositor offers it, X11
otherwise. Both are reported by `kotodex doctor`.
