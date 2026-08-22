# Which compositors the overlay works on

The overlay draws the line feed *above* a fullscreen game and takes clicks only
where it has drawn. Two mechanisms can do that, and which is available is a
property of the compositor, not of the app:

- **layer-shell** (`wlr-layer-shell`, via `org.kde.layershell`) — a surface on
  the overlay layer is above everything by protocol. This is what
  `layer-overlay/` uses today.
- **X11 always-on-top** (`_NET_WM_STATE_ABOVE` + XShape input region), running
  on XWayland under a Wayland session. Works wherever XWayland does.

## Results

| session | stays above fullscreen | click-through | xdotool geometry | verdict |
|---|---|---|---|---|
| KDE Wayland | yes (layer-shell) | yes | yes | supported — the development target |
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

Still to check on GNOME: whether the XShape input region lets clicks through
where the page has not drawn.

## What this means for the release

The fullscreen overlay is not limited to layer-shell compositors. GNOME —
the largest single desktop — works through the X11 backend, which makes T4.8
required rather than optional, and the "90% of desktop Linux" claim defensible
rather than something to narrow.

Backend selection at startup: layer-shell where the compositor offers it, X11
otherwise. Both are reported by `kotodex doctor`.
