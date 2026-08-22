# Which compositors the overlay works on

The overlay draws the line feed *above* a fullscreen game and takes clicks only
where it has drawn. Two different mechanisms can do that, and which one is
available is a property of the compositor, not of the app:

- **layer-shell** (`wlr-layer-shell`, via `org.kde.layershell`) — a surface on
  the overlay layer is above everything by protocol. This is what
  `layer-overlay/` uses.
- **X11 always-on-top** (`_NET_WM_STATE_ABOVE` + XShape input region) — the
  fallback for sessions with no layer-shell.

## Results

| session | stays above fullscreen | click-through | xdotool geometry | verdict |
|---|---|---|---|---|
| KDE Wayland | yes (layer-shell) | yes | yes | supported — the development target |
| Hyprland | expected yes (layer-shell) | — | — | untested, wlroots implements the protocol |
| GNOME Wayland | **no** | — | yes | not supported for fullscreen |
| GNOME Xorg | — | — | — | no such session; GNOME has dropped X11 |
| KDE Xorg | — | — | — | not installed here (`plasma-x11-session`) |

## GNOME Wayland, in detail

Tested with `scripts/spike/stack-test.py` (a frameless `Qt.WindowStaysOnTopHint`
window with `setMask`) over `mpv --gpu-context=x11 --fs` and
`glxgears -fullscreen`.

The probe is drawn on top at first, and **drops behind the moment the fullscreen
window is clicked** — mutter raises a focused fullscreen window above an
always-on-top XWayland window, and there is no window-manager hint that opts
out. The X11 fallback therefore cannot carry the overlay on GNOME.

There is no layer-shell path either: mutter does not implement
`wlr-layer-shell`, and it is not a matter of a missing package.

`xdotool search --name … getwindowgeometry` **does** work — Wine and Proton
windows are XWayland, so window tracking is fine under GNOME. Only the stacking
fails.

## What this means for the release

Per the T0.4 decision gate, the README says:

> Kotodex installs and runs on any modern desktop Linux. The **fullscreen**
> overlay needs a compositor with layer-shell — KDE Plasma, or a wlroots
> compositor such as Hyprland or Sway. On GNOME, run the game windowed and read
> the feed beside it in a browser.

That sentence is what the test supports, and it does not get softened later.

Windowed reading on GNOME is not a degraded fallback bolted on for this: `#read`
in a browser is a first-class surface with the same feed, the same dictionary
popup and Yomitan over it.
