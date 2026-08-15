#!/usr/bin/env python3
"""The line being read, as a layer-shell strip over the VN, fullscreen included.

KWin puts a `zwlr_layer_shell_v1` overlay surface above fullscreen windows, so
the line can sit on the game instead of beside it.

The page is `overlay.html` beside this file — the newest line, and a dictionary
popup for the word clicked in it. Yomitan does not run here (QtWebEngine loads
no extensions), so the popup is the page's own.

read-stats serves that file, from this directory, and is the overlay's backend:
the page calls eight `/api` routes for the line stream, the dictionary, the
ledger and the card. Nothing of it can be answered locally — the dictionary and
the ledger are jp-core's — so the overlay is a client of read-stats and starting
without it gets a warning, not a failure.

**The surface is clickable only where it has drawn something.** The page pushes
the box it has drawn the line in, and the popup's own box, over a WebChannel,
and those become the input region: a click on a word looks it up while a click
anywhere else reaches the VN and advances the line. There is no mode to switch.

That leaves nothing to dismiss the popup with, since a click outside never
arrives here: it closes on the same word clicked again, on another word, or on
the next line landing — which is what the click that missed it just caused.

`SIGUSR1` makes the *whole* surface take input, for the rare case of wanting to
select text rather than advance the VN. Bind it to a KDE shortcut:

    pkill -USR1 -f vn-overlay.py

`SIGUSR2` toggles ghost mode: the line is laid over the game's own text and then
drawn invisibly, so what is read is the game's typesetting and all this adds is
the status tint per word and somewhere to click. It needs the line to sit on the
game's text to the pixel, which is what the window geometry above is for — see
`--text-x` and friends in `overlay.html` for the per-game measurements.

    pkill -USR2 -f vn-overlay.py

`--mobile` is the overlay read off a phone: everything at 1.75x, the line on
the bottom edge, and the popup carrying known / unknown / mine buttons, since
driving the PC's mouse from the phone leaves no side buttons for them. The
strip grows with the type; `VN_OVERLAY_HEIGHT` still wins if it is set.

    VN_OVERLAY_URL      page to show      (default overlay.html, over read-stats)
    JP_TOOLS_ANKI_URL   AnkiConnect       (default http://localhost:8765)
    VN_OVERLAY_HEIGHT   strip height, px  (default 300, 525 with --mobile)
    VN_OVERLAY_BG       backdrop alpha    (default 0.82)
    VN_OVERLAY_FONT     font for the line (vn-overlay.sh sets DNP Shuei Mincho
                        Pr6; unset falls back to the page's Noto Sans CJK JP)
"""

import argparse
import json
import os
import shutil
import signal
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path
from urllib.parse import quote, urlsplit

# Turns every window of this process into a layer surface. Must be set before
# QGuiApplication resolves the platform plugin.
os.environ.setdefault("QT_WAYLAND_SHELL_INTEGRATION", "layer-shell")

from PySide6.QtCore import QFile, QIODevice, QObject, QTimer, QUrl, Signal, Slot
from PySide6.QtGui import QGuiApplication, QRegion
from PySide6.QtQml import QQmlApplicationEngine
# Registers the qrc that webchannel_script() reads.
from PySide6.QtWebChannel import QWebChannel  # noqa: F401
from PySide6.QtWebEngineCore import QWebEngineScript
from PySide6.QtWebEngineQuick import QtWebEngineQuick

DEFAULT_URL = "http://localhost:3200/overlay/overlay.html"
ANKI_URL = os.environ.get("JP_TOOLS_ANKI_URL", "http://localhost:8765")
# Where the page's localStorage lives between runs — see Overlay.qml's profile.
STORAGE = (
    Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share") / "vn-mine/overlay"
)


def check_dependencies(page_url: str) -> None:
    """Say which of the overlay's dependencies are down, and keep going.

    Warn rather than exit, both times: `EventSource` reconnects on its own, so
    an overlay started before read-stats catches up when read-stats arrives,
    and Anki is only needed at the moment a word is mined. Exiting would make
    the start order matter when it does not.

    Textractor is deliberately absent. The page already reports it live from
    `settings.vn_logger_heartbeat`, in `#warn`, which stays right when it stops
    mid-session — a check here would be a second answer that goes stale.
    """
    origin = urlsplit(page_url)
    api = f"{origin.scheme}://{origin.netloc}"

    try:
        with urllib.request.urlopen(f"{api}/api/settings", timeout=2) as r:
            json.load(r)
    except (urllib.error.URLError, OSError, ValueError) as e:
        print(
            f"read-stats not answering on {api} ({e}) — the strip stays empty "
            "until it does. scripts/start-all.sh start read-stats",
            file=sys.stderr,
        )

    request = urllib.request.Request(
        ANKI_URL,
        data=json.dumps({"action": "version", "version": 6}).encode(),
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=2) as r:
            json.load(r)
    except (urllib.error.URLError, OSError, ValueError) as e:
        print(
            f"Anki not answering on {ANKI_URL} ({e}) — reading and lookups work, "
            "mining will fail",
            file=sys.stderr,
        )


GEOMETRY_POLL_MS = 300


class Overlay(QObject):
    """Owns which parts of the strip take clicks, and where the game is."""

    #: The game window as `x, y, width, height`, or a zero rectangle when it
    #: cannot be found. The page lays the line over the game's own text off
    #: this, so a move or a resize carries the line along instead of leaving it
    #: measured against a screen the game no longer fills.
    geometry = Signal(int, int, int, int)

    #: Draw the marks but not the words — see `SIGUSR2` in the module docstring.
    ghostToggled = Signal()

    def __init__(self) -> None:
        super().__init__()
        self._window = None
        self.interactive = False
        self._hits = []
        self._name = ""
        self._rect = None
        self._xdotool = shutil.which("xdotool")
        self._probe = QTimer()
        self._probe.setInterval(GEOMETRY_POLL_MS)
        self._probe.timeout.connect(self._poll_geometry)

    @Slot(str)
    def setWindowName(self, name: str) -> None:
        """Which window is the game, as read-stats holds it for the current work.

        Pushed from the page rather than read here: the name is a column on the
        work, and a copy in this process would be the one left stale when the
        work changes. Polled rather than watched because there is no X event for
        "a window matching this name appeared" that is cheaper than asking.
        """
        name = (name or "").strip()
        if name == self._name:
            return
        self._name = name
        self._rect = None
        if name and self._xdotool:
            self._poll_geometry()
            self._probe.start()
        else:
            self._probe.stop()
            self.geometry.emit(0, 0, 0, 0)

    def _poll_geometry(self) -> None:
        rect = self._window_rect()
        if rect == self._rect:
            return
        self._rect = rect
        self.geometry.emit(*(rect or (0, 0, 0, 0)))

    def _window_rect(self):
        # Wine and Proton VNs are XWayland windows, so they stay addressable
        # through X under a Wayland session — the same reason vn-capture.sh
        # finds the window this way. A Wayland-native game has no equivalent,
        # and the page falls back to placing the line against the screen.
        try:
            out = subprocess.run(
                [self._xdotool, "search", "--name", self._name,
                 "getwindowgeometry", "--shell", "%1"],
                capture_output=True,
                text=True,
                timeout=2,
            )
        except (OSError, subprocess.SubprocessError):
            return None
        if out.returncode != 0:
            return None
        fields = {}
        for line in out.stdout.splitlines():
            key, _, value = line.partition("=")
            if value.lstrip("-").isdigit():
                fields[key] = int(value)
        if not {"X", "Y", "WIDTH", "HEIGHT"} <= fields.keys():
            return None
        return (fields["X"], fields["Y"], fields["WIDTH"], fields["HEIGHT"])

    def attach(self, window) -> None:
        self._window = window
        # The compositor configures a layer surface after the window is
        # created, and a Plasma panel's exclusive zone shrinks it further, so
        # the height here is not the final one. Recompute when it settles.
        window.heightChanged.connect(self.apply)
        self.apply()

    @Slot(list)
    def setHits(self, flat) -> None:
        """Take what the page says is clickable, as flat `x, y, w, h, ...`."""
        v = [int(n) for n in flat]
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
        # The region reaches the compositor on the surface's next commit, and a
        # page that has finished painting schedules no further frame — so
        # without this the popup can sit on screen taking no clicks until
        # something else happens to repaint.
        self._window.requestUpdate()
        if os.environ.get("VN_OVERLAY_DEBUG"):
            rects = " ".join(f"{r.x()},{r.y()} {r.width()}x{r.height()}" for r in region)
            print(f"mask [{len(self._hits)}] {rects}", flush=True)
        self._window.setProperty("interactive", self.interactive)


class Surface:
    """Keeps a layer surface on screen across output changes.

    A layer surface belongs to an output. When that output goes away — which is
    what Moonlight/Sunshine disconnecting does, it removes the virtual one KWin
    added — the compositor closes the surface, and with it the process's only
    window. Nothing errors and nothing is logged; Qt just runs out of windows.
    So: don't quit on that, and build a new window instead. `Screen.width` is
    read once per window, so a rebuild is also what picks up a mode change.
    """

    def __init__(self, app, overlay, qml, context) -> None:
        self._app = app
        self._overlay = overlay
        self._qml = qml
        self._context = context
        self._engine = None
        self._quitting = False
        app.setQuitOnLastWindowClosed(False)
        app.aboutToQuit.connect(self._stop)

        # Output churn arrives as a burst of signals; rebuild once after it.
        self._rebuild = QTimer()
        self._rebuild.setSingleShot(True)
        self._rebuild.setInterval(500)
        self._rebuild.timeout.connect(self.build)
        for signal_ in (app.screenAdded, app.screenRemoved, app.primaryScreenChanged):
            signal_.connect(self._schedule)

    def _stop(self) -> None:
        self._quitting = True
        self._rebuild.stop()

    def _closed(self, window) -> None:
        if window is self._engine.rootObjects()[0] and not window.isVisible():
            self._schedule()

    def _schedule(self, *_) -> None:
        if not self._quitting:
            self._rebuild.start()

    def build(self) -> bool:
        if self._quitting:
            return True
        old = self._engine
        engine = QQmlApplicationEngine()
        ctx = engine.rootContext()
        for name, value in self._context.items():
            ctx.setContextProperty(name, value)
        engine.load(QUrl.fromLocalFile(self._qml))
        if not engine.rootObjects():
            self._engine = old
            return False
        self._engine = engine
        window = engine.rootObjects()[0]
        # The compositor closing the surface shows up here as the window going
        # invisible, with no screen signal to go with it.
        window.visibleChanged.connect(lambda *_: self._closed(window))
        self._overlay.attach(window)
        if old is not None:
            old.deleteLater()
        return True


def webchannel_script() -> QWebEngineScript:
    """Qt's own `qwebchannel.js`, for the view to run before the page does.

    The page has no reason to carry a copy: it ships inside QtWebChannel as a
    qrc resource, so the version in the page always matches the one on this
    side of the channel. Built here rather than in QML, where WebEngineScript
    is a value type and not creatable as an element.
    """
    f = QFile(":/qtwebchannel/qwebchannel.js")
    f.open(QIODevice.OpenModeFlag.ReadOnly)
    script = QWebEngineScript()
    script.setName("qwebchannel")
    script.setSourceCode(bytes(f.readAll()).decode())
    script.setInjectionPoint(QWebEngineScript.InjectionPoint.DocumentCreation)
    script.setWorldId(QWebEngineScript.ScriptWorldId.MainWorld)
    return script


def main() -> int:
    # Qt's event loop never returns to the interpreter, so a Python-level SIGINT
    # handler would never run and Ctrl+C would do nothing. The C default kills
    # the process, and the terminal signals the whole group, so the WebEngine
    # helper processes go with it.
    signal.signal(signal.SIGINT, signal.SIG_DFL)

    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--mobile", action="store_true", help="1.75x, with touch buttons")
    args, qt_args = ap.parse_known_args()
    scale = 1.75 if args.mobile else 1

    QtWebEngineQuick.initialize()  # has to precede QGuiApplication
    app = QGuiApplication([sys.argv[0], *qt_args])

    height = int(os.environ.get("VN_OVERLAY_HEIGHT", 300 * scale))
    url = os.environ.get("VN_OVERLAY_URL", DEFAULT_URL)
    if url == DEFAULT_URL:
        url += f"?bg={os.environ.get('VN_OVERLAY_BG', '0.82')}&h={height}&scale={scale}"
        if args.mobile:
            url += "&mobile=1"
        font = os.environ.get("VN_OVERLAY_FONT")
        if font:
            url += f"&font={quote(font)}"

    check_dependencies(url)

    overlay = Overlay()
    surface = Surface(
        app,
        overlay,
        str(Path(__file__).resolve().parent / "Overlay.qml"),
        {
            "overlay": overlay,
            "overlayUrl": url,
            "overlayHeight": height,
            "overlayStorage": str(STORAGE),
            "overlayWebChannelScript": webchannel_script(),
        },
    )
    if not surface.build():
        print("QML failed to load", file=sys.stderr)
        return 1

    signal.signal(signal.SIGUSR1, lambda *_: overlay.toggle())
    signal.signal(signal.SIGUSR2, lambda *_: overlay.ghostToggled.emit())
    # Python only runs a signal handler between bytecodes and Qt's event loop
    # is C, so nothing above would ever land without a tick that returns to the
    # interpreter. It has no other work.
    wake = QTimer()
    wake.start(200)
    wake.timeout.connect(lambda: None)

    return app.exec()


if __name__ == "__main__":
    sys.exit(main())
