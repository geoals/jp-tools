"""A web page as a `zwlr_layer_shell_v1` overlay surface, above fullscreen windows.

Nothing here knows what the page is for. Give it a URL and it draws that page
over everything, clickable only where the page says it has drawn something —
every click elsewhere reaches the window underneath.

Three pieces, and the page needs all three:

- [`Overlay`] is the object the page talks to over a WebChannel. The page
  pushes the rectangles it has drawn; those become the input region.
- [`Surface`] keeps a window on screen across output changes. A layer surface
  belongs to an output, so losing one closes the surface with no error.
- [`run`] wires both to a `QGuiApplication` and the two signals.

The page is expected to run `qwebchannel.js` — [`webchannel_script`] injects
Qt's own copy — and to connect to the object registered as `shell`:

    shell.setHits([x, y, w, h, ...])   what takes clicks, flat
    shell.setWindowName(name)          track this window's rectangle
    shell.geometry(x, y, w, h)         where it is now, or zeros
    shell.userToggled()                SIGUSR2 reached the page

`SIGUSR1` makes the *whole* surface take input, for selecting text rather than
clicking through. `SIGUSR2` is the page's to define. Both are sent by name:

    pkill -USR1 -f <the script that called run()>

Needs PySide6, qt6-webengine and layer-shell-qt as **system packages** — a venv
build of PySide6 carries no `org.kde.layershell`.
"""

import os
import shutil
import signal
import subprocess
import sys
from pathlib import Path

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

QML = str(Path(__file__).resolve().parent / "Overlay.qml")

GEOMETRY_POLL_MS = 300


class Overlay(QObject):
    """Owns which parts of the surface take clicks, and where the tracked window is."""

    #: The tracked window as `x, y, width, height`, or a zero rectangle when it
    #: cannot be found. A page that lays itself over another window's content
    #: reads this, so a move or a resize carries it along instead of leaving it
    #: measured against a screen that window no longer fills.
    geometry = Signal(int, int, int, int)

    #: SIGUSR2 reached the page. What it means is the page's to decide.
    userToggled = Signal()

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
        """Which window to track, by title substring.

        Pushed from the page rather than read here: the page is where that name
        comes from, and a copy in this process would be the one left stale when
        it changes. Polled rather than watched because there is no X event for
        "a window matching this name appeared" that is cheaper than asking.
        """
        name = (name or "").strip()
        self._name = name
        # Answer every push, not only a new name. The page pushes on each
        # channel connect, and a reloaded page holds no geometry — dropping the
        # repeat would leave it placed against the screen until the tracked
        # window next moved.
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
        # XWayland windows stay addressable through X under a Wayland session,
        # which is what makes this work for Wine and Proton games. A
        # Wayland-native window has no equivalent, and the page falls back to
        # placing itself against the screen.
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
        # without this a freshly drawn element can sit on screen taking no
        # clicks until something else happens to repaint.
        self._window.requestUpdate()
        if os.environ.get("LAYER_OVERLAY_DEBUG"):
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


def run(url: str, *, scope: str, storage, qt_args=()) -> int:
    """Show `url` as an overlay surface and run until it is killed.

    `scope` names the surface to the compositor and names the page's persistent
    storage, so window rules and anything in `localStorage` survive a restart.
    `storage` is where that storage lives on disk.
    """
    # Qt's event loop never returns to the interpreter, so a Python-level SIGINT
    # handler would never run and Ctrl+C would do nothing. The C default kills
    # the process, and the terminal signals the whole group, so the WebEngine
    # helper processes go with it.
    signal.signal(signal.SIGINT, signal.SIG_DFL)

    QtWebEngineQuick.initialize()  # has to precede QGuiApplication
    app = QGuiApplication([sys.argv[0], *qt_args])

    overlay = Overlay()
    surface = Surface(
        app,
        overlay,
        QML,
        {
            "overlay": overlay,
            "overlayUrl": url,
            "overlayScope": scope,
            "overlayStorage": str(storage),
            "overlayWebChannelScript": webchannel_script(),
        },
    )
    if not surface.build():
        print("QML failed to load", file=sys.stderr)
        return 1

    signal.signal(signal.SIGUSR1, lambda *_: overlay.toggle())
    signal.signal(signal.SIGUSR2, lambda *_: overlay.userToggled.emit())
    # Python only runs a signal handler between bytecodes and Qt's event loop
    # is C, so nothing above would ever land without a tick that returns to the
    # interpreter. It has no other work.
    wake = QTimer()
    wake.start(200)
    wake.timeout.connect(lambda: None)

    return app.exec()
