"""A web page as an overlay surface, above fullscreen windows.

Nothing here knows what the page is for. Give it a URL and it draws that page
over everything, clickable only where the page says it has drawn something —
every click elsewhere reaches the window underneath.

Two backends put it there — layer-shell where the compositor offers it, an
always-on-top XWayland window otherwise. [`backend`] picks between them and the
page cannot tell which it got. Three pieces, and the page needs all three:

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

import backend
from xshape import InputRegion

# Both the platform plugin and the shell integration are read once, when
# QGuiApplication is constructed, so the backend has to be settled before any Qt
# import that could pull one in.
BACKEND, BACKEND_REASON = backend.choose()
backend.apply_environment(BACKEND)

#: Shrink the surface onto the tracked window instead of covering the whole
#: output. Layer-shell only: it is the surface's own size that moves, which is
#: something only the layer-shell protocol lets this ask for.
#: `LAYER_OVERLAY_CONFINE=0` goes back to a surface over everything.
CONFINE = (
    os.environ.get("LAYER_OVERLAY_CONFINE", "1") != "0"
    and BACKEND == backend.LAYER_SHELL
)

from PySide6.QtCore import QFile, QIODevice, QObject, QTimer, QUrl, Signal, Slot
from PySide6.QtGui import QGuiApplication, QRegion
from PySide6.QtQml import QQmlApplicationEngine
# Registers the qrc that webchannel_script() reads.
from PySide6.QtWebChannel import QWebChannel  # noqa: F401
from PySide6.QtWebEngineCore import QWebEngineScript
from PySide6.QtWebEngineQuick import QtWebEngineQuick

_HERE = Path(__file__).resolve().parent
QML = str(_HERE / ("Overlay.qml" if BACKEND == backend.LAYER_SHELL else "OverlayX11.qml"))

GEOMETRY_POLL_MS = 300


def _rects(region):
    return [(r.x(), r.y(), r.width(), r.height()) for r in region]


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
        #: Minimised: off screen but still running, and kept across the window
        #: rebuilds that an output change causes.
        self.hidden = False
        self._hits = []
        self._name = ""
        self._rect = None
        self._confined = False
        self._xdotool = shutil.which("xdotool")
        # Only the X11 backend needs one: under Wayland the mask already means
        # the input region, and opening an X connection would be pointless.
        self._input = InputRegion() if BACKEND == backend.X11 else None
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
            # Through _confine, which has the surface to give back: a name
            # cleared while shrunk onto a window would otherwise leave the
            # surface that size with the page no longer placing anything
            # against it, and everything the page then puts outside those
            # bounds is clipped away rather than drawn.
            if CONFINE:
                self._confine(None)
            else:
                self.geometry.emit(0, 0, 0, 0)

    def _poll_geometry(self) -> None:
        rect = self._window_rect()
        if rect == self._rect:
            return
        self._rect = rect
        if CONFINE:
            self._confine(rect)
            return
        self.geometry.emit(*(self._to_surface(rect) if rect else (0, 0, 0, 0)))

    def _confine(self, rect) -> None:
        """Shrink the surface onto `rect`, or back over the whole output.

        The page is then told `(0, 0, w, h)`: the surface starts where the
        window does, so the offset `_to_surface` exists to apply is zero by
        construction and the page's own origin is the window's.

        What this does *not* buy is being covered by other windows. A layer
        surface is above them by protocol wherever it is — this only stops it
        being over the parts of the screen the window does not occupy.
        """
        # Before the window check, both of them: giving the surface back is
        # what has to happen even when there is no window to do it to, and the
        # page is owed the zero rectangle either way.
        if rect is None:
            self._inset(0, 0, 0, 0)
            self.geometry.emit(0, 0, 0, 0)
            return
        if self._window is None:
            return
        screen = self._window.screen()
        if screen is None:
            return
        # X answers in device pixels; a layer surface's margins, like every
        # other length Qt takes, are logical ones.
        scale = self._window.devicePixelRatio() or 1.0
        x, y, w, h = (round(v / scale) for v in rect)
        area = screen.geometry()
        left = max(x - area.x(), 0)
        top = max(y - area.y(), 0)
        self._inset(
            left, top,
            max(area.width() - left - w, 0),
            max(area.height() - top - h, 0),
        )
        self.geometry.emit(0, 0, w, h)

    def _inset(self, left: int, top: int, right: int, bottom: int) -> None:
        """The four margins the QML binds the surface's own to."""
        self._confined = any((left, top, right, bottom))
        if self._window is None:
            return
        for name, value in (
            ("insetLeft", left), ("insetTop", top),
            ("insetRight", right), ("insetBottom", bottom),
        ):
            self._window.setProperty(name, value)
        # The surface is reconfigured to a new size, and the mask is in its
        # coordinates — so what was clickable has moved.
        self.apply()

    def _to_surface(self, rect):
        """The tracked window in the page's own coordinates.

        A layer surface covers the output, so this is identity there. A
        window manager shrinks an X11 surface to the *work area* instead, and a
        panel then offsets it — leaving the page to place everything against a
        screen origin its surface does not start at.
        """
        x, y, w, h = rect
        if self._window is None:
            return (x, y, w, h)
        return (x - self._window.x(), y - self._window.y(), w, h)

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
        if self.hidden:
            window.setVisible(False)
        self._window = window
        # The compositor configures a layer surface after the window is
        # created, and a Plasma panel's exclusive zone shrinks it further, so
        # the height here is not the final one. Recompute when it settles.
        window.heightChanged.connect(self.apply)
        self.apply()
        # A rebuilt window carries none of the insets the old one had, and the
        # rectangle has not changed, so nothing else would re-apply them.
        if CONFINE and self._rect is not None:
            self._confine(self._rect)

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

    @Slot()
    def minimise(self) -> None:
        """Off screen, still running. The window is rebuilt on output changes,
        so hiding it is a flag the rebuild honours rather than a one-off."""
        self.hidden = True
        if self._window is not None:
            self._window.setVisible(False)

    @Slot()
    def restore(self) -> None:
        self.hidden = False
        if self._window is not None:
            self._window.setVisible(True)

    @Slot()
    def quit(self) -> None:
        """The page asking to be closed. Exit 0 says *deliberate*, which is how
        whatever started this tells a close apart from a crash."""
        from PySide6.QtGui import QGuiApplication

        app = QGuiApplication.instance()
        if app is not None:
            app.exit(0)

    def apply(self) -> None:
        if self._window is None or self._window.height() <= 0:
            return
        # Qt maps a window mask onto wl_surface.set_input_region under Wayland
        # and onto an XShape input region under X11, so one call covers both
        # backends. Both branches pass a non-empty region on purpose: an empty
        # mask means "the whole surface takes input", which is the opposite of
        # passing clicks through.
        if self.interactive:
            region = QRegion(0, 0, self._window.width(), self._window.height())
        elif self._hits:
            region = QRegion()
            for x, y, w, h in self._hits:
                region = region.united(QRegion(x, y, max(w, 1), max(h, 1)))
        else:
            # One pixel, not nothing: an *empty* mask means the whole surface
            # takes input, which is the opposite of what a page reporting no
            # clickable area should do. The X11 input region reads an empty list
            # the way it looks, so this costs it only one dead pixel.
            region = QRegion(0, 0, 1, 1)
        if self._input is not None and self._input.available:
            self._input.apply(int(self._window.winId()), _rects(region))
        else:
            self._window.setMask(region)
        # The region reaches the compositor on the surface's next commit, and a
        # page that has finished painting schedules no further frame — so
        # without this a freshly drawn element can sit on screen taking no
        # clicks until something else happens to repaint.
        self._window.requestUpdate()
        if os.environ.get("LAYER_OVERLAY_DEBUG"):
            rects = " ".join(f"{x},{y} {w}x{h}" for x, y, w, h in _rects(region))
            print(
                f"mask [{len(self._hits)}] "
                f"in {self._window.width()}x{self._window.height()} {rects}",
                flush=True,
            )
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

    print(f"backend: {BACKEND} ({BACKEND_REASON})", flush=True)

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
