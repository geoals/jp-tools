#!/usr/bin/env python3
"""The line being read, as a layer-shell strip over the VN, fullscreen included.

KWin puts a `zwlr_layer_shell_v1` overlay surface above fullscreen windows, so
the line can sit on the game instead of beside it.

The page is read-stats' `/static/overlay.html` — the newest line, and a
dictionary popup for the word clicked in it. Yomitan does not run here
(QtWebEngine loads no extensions), so the popup is the page's own.

**The surface is clickable only where its words are.** The page reports a
rectangle per word and the popup's own box, and those become the input region,
so a click on a word looks it up while a click anywhere else — including the
empty parts of the strip — reaches the VN and advances the line. There is no
mode to switch.

That leaves nothing to dismiss the popup with, since a click outside never
arrives here: it closes on the same word clicked again, on another word, or on
the next line landing — which is what the click that missed it just caused.

`SIGUSR1` makes the *whole* surface take input, for the rare case of wanting to
select text rather than advance the VN. Bind it to a KDE shortcut:

    pkill -USR1 -f vn-overlay.py

    VN_OVERLAY_URL      page to show      (default read-stats' overlay page)
    VN_OVERLAY_HEIGHT   strip height, px  (default 300)
    VN_OVERLAY_BG       backdrop alpha    (default 0.75)
"""

import os
import signal
import sys
from pathlib import Path

# Turns every window of this process into a layer surface. Must be set before
# QGuiApplication resolves the platform plugin.
os.environ.setdefault("QT_WAYLAND_SHELL_INTEGRATION", "layer-shell")

from PySide6.QtCore import QObject, QTimer, QUrl, Slot
from PySide6.QtGui import QGuiApplication, QRegion
from PySide6.QtQml import QQmlApplicationEngine
from PySide6.QtWebEngineQuick import QtWebEngineQuick

DEFAULT_URL = "http://localhost:3200/static/overlay.html"


class Overlay(QObject):
    """Owns which parts of the strip take clicks."""

    def __init__(self) -> None:
        super().__init__()
        self._window = None
        self.interactive = False
        self._hits = []

    def attach(self, window) -> None:
        self._window = window
        # The compositor configures a layer surface after the window is
        # created, and a Plasma panel's exclusive zone shrinks it further, so
        # the height here is not the final one. Recompute when it settles.
        window.heightChanged.connect(self.apply)
        self.apply()

    def poll(self) -> None:
        """Take whatever the page last reported as clickable."""
        if self._window is None:
            return
        flat = self._window.property("hitRects") or []
        try:
            v = [int(n) for n in flat]
        except TypeError:
            return  # the page has not defined __hitRects yet
        hits = [tuple(v[i : i + 4]) for i in range(0, len(v) - 3, 4)]
        if hits == self._hits:
            return
        self._hits = hits
        self.apply()

    @Slot()
    def toggle(self) -> None:
        self.interactive = not self.interactive
        self.apply()

    def apply(self) -> None:
        if self._window is None or self._window.height() <= 0:
            return
        # Qt maps a window mask onto wl_surface.set_input_region. Both branches
        # pass a non-empty region on purpose: an empty mask means "the whole
        # surface takes input", which is the opposite of passing clicks through.
        if self.interactive:
            region = QRegion(0, 0, self._window.width(), self._window.height())
        elif self._hits:
            region = QRegion()
            for x, y, w, h in self._hits:
                region = region.united(QRegion(x, y, max(w, 1), max(h, 1)))
        else:
            # One pixel, not nothing: an *empty* mask means the whole surface
            # takes input, which is the opposite of what a page reporting no
            # clickable area should do.
            region = QRegion(0, 0, 1, 1)
        self._window.setMask(region)
        if os.environ.get("VN_OVERLAY_DEBUG"):
            rects = " ".join(f"{r.x()},{r.y()} {r.width()}x{r.height()}" for r in region)
            print(f"mask [{len(self._hits)}] {rects}", flush=True)
        self._window.setProperty("interactive", self.interactive)


def main() -> int:
    # Qt's event loop never returns to the interpreter, so a Python-level SIGINT
    # handler would never run and Ctrl+C would do nothing. The C default kills
    # the process, and the terminal signals the whole group, so the WebEngine
    # helper processes go with it.
    signal.signal(signal.SIGINT, signal.SIG_DFL)

    QtWebEngineQuick.initialize()  # has to precede QGuiApplication
    app = QGuiApplication(sys.argv)

    height = int(os.environ.get("VN_OVERLAY_HEIGHT", "300"))
    url = os.environ.get("VN_OVERLAY_URL", DEFAULT_URL)
    if url == DEFAULT_URL:
        url += f"?bg={os.environ.get('VN_OVERLAY_BG', '0.75')}&h={height}"

    overlay = Overlay()
    engine = QQmlApplicationEngine()
    ctx = engine.rootContext()
    ctx.setContextProperty("overlay", overlay)
    ctx.setContextProperty("overlayUrl", url)
    ctx.setContextProperty("overlayHeight", height)

    engine.load(QUrl.fromLocalFile(str(Path(__file__).resolve().parent / "Overlay.qml")))
    if not engine.rootObjects():
        print("QML failed to load", file=sys.stderr)
        return 1
    overlay.attach(engine.rootObjects()[0])

    signal.signal(signal.SIGUSR1, lambda *_: overlay.toggle())
    # Two jobs, one tick: Python only runs a signal handler between
    # bytecodes and Qt's loop is C, so this is what lets SIGUSR1 land — and
    # it is where the page's word rectangles are picked up.
    wake = QTimer()
    wake.start(25)
    wake.timeout.connect(overlay.poll)

    return app.exec()


if __name__ == "__main__":
    sys.exit(main())
