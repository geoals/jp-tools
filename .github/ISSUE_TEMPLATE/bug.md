---
name: Something is broken
about: A part of Kotodex does not do what it says
labels: bug
---

**What happened, and what you expected instead.**

**`kotodex doctor` output** — paste the whole thing. Most reports are answered
by one row of it:

```
$ kotodex doctor
```

**Which compositor** (KDE Wayland, GNOME Wayland, Hyprland, X11 …) and whether
the game was fullscreen.

**Anything in the logs** — `journalctl --user -u kotodex-capture -n 50`, or the
terminal Kotodex was launched from.
