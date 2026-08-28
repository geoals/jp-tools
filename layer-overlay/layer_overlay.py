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
    shell.openUrl(url)                 open an http(s) link in the browser
    shell.quit()                       close; `run` returns QUIT_REQUESTED

`SIGUSR1` makes the *whole* surface take input, for selecting text rather than
clicking through. `SIGUSR2` is the page's to define. Both are sent by name:

    pkill -USR1 -f <the script that called run()>

Needs PySide6 with Qt WebEngine. The layer-shell backend needs `layer-shell-qt`
too, and all of them as **system packages** — a pip PySide6 carries its own Qt,
which cannot load a system `org.kde.layershell`. [`backend`] checks for that and
answers x11, so a venv build still runs.
"""

import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

import backend

_START = time.monotonic()


def since_start() -> str:
    """Seconds since this module was imported, for the startup lines.

    A reader reporting a slow start can say nothing useful about which part was
    slow, and the answer is never the same part twice. Every line printed before
    the surface exists carries this, so one log says where the time went.
    """
    return f"+{time.monotonic() - _START:6.2f}s"

# Both the platform plugin and the shell integration are read once, when
# QGuiApplication is constructed, so the backend has to be settled before any Qt
# import that could pull one in.
BACKEND, BACKEND_REASON = backend.choose()
backend.apply_environment(BACKEND)

# Which pair of these is importable is the platform's answer, not a choice: the
# X11 modules need libX11 and the Windows ones need user32. Both pairs answer the
# same two questions - where the tracked window is, and what takes clicks - so
# everything below is written against the interface rather than against either.
if BACKEND == backend.WINDOWS:
    import wininput as inputregion
    import winwatch as windowwatch
else:
    import xshape as inputregion
    import xwatch as windowwatch

from PySide6.QtCore import (
    QFile, QIODevice, QObject, QSocketNotifier, QTimer, QUrl, Signal, Slot,
)
from PySide6.QtGui import QDesktopServices, QGuiApplication, QRegion
from PySide6.QtQml import QQmlApplicationEngine
# Registers the qrc that webchannel_script() reads.
from PySide6.QtWebChannel import QWebChannel  # noqa: F401
from PySide6.QtWebEngineCore import QWebEngineScript
from PySide6.QtWebEngineQuick import QtWebEngineQuick

_HERE = Path(__file__).resolve().parent
QML = str(_HERE / ("Overlay.qml" if BACKEND == backend.LAYER_SHELL else "OverlayWindow.qml"))

#: [`run`] returns this when the page called `shell.quit()`. What it means is
#: the caller's to decide — this only distinguishes it from a clean exit the
#: page did not ask for.
QUIT_REQUESTED = 3

#: Only for the fallback, and only for *finding* a window. Where X can be
#: watched, a window that has been found reports its own moves — see [`xwatch`].
GEOMETRY_POLL_MS = 300

#: With the watcher, the timer is a safety net rather than the mechanism: it
#: catches a window whose title changes into a match, which is the one thing
#: no event here subscribes to.
DISCOVERY_POLL_MS = 1000


def _rects(region, scale=1.0):
    return [
        (
            round(r.x() * scale),
            round(r.y() * scale),
            round(r.width() * scale),
            round(r.height() * scale),
        )
        for r in region
    ]


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
        #: Whether the page has ever pushed a region. The first push is the
        #: moment the overlay becomes clickable, which is what a reader
        #: experiences as it being *there* — the surface exists well before it.
        self._reported = False
        self._name = ""
        self._rect = None
        self._xdotool = shutil.which("xdotool")
        # Events where X can be watched, the subprocess and a timer where it
        # cannot. The page cannot tell which it got; the difference is whether
        # the line arrives with the window or up to an interval behind it.
        self._watch = windowwatch.WindowWatch()
        self._notifier = None
        if self._watch.available and self._watch.fd is not None:
            self._notifier = QSocketNotifier(
                self._watch.fd, QSocketNotifier.Type.Read
            )
            self._notifier.activated.connect(self._on_x_ready)
        # Windows delivers its window events through the thread's message queue,
        # which Qt is already pumping, so there is no descriptor to wait on and
        # the watcher calls back instead.
        if self._watch.available and self._watch.fd is None:
            self._watch.on_change = self._poll_geometry
        # Every backend but layer-shell, where the mask already means the input
        # region and there is nothing to set by hand.
        self._input = (
            inputregion.InputRegion() if BACKEND != backend.LAYER_SHELL else None
        )
        # Windows has no input region, so the one it emulates has to be
        # re-evaluated as the cursor moves rather than set and forgotten.
        self._cursor = None
        if BACKEND == backend.WINDOWS:
            self._cursor = QTimer()
            self._cursor.setInterval(inputregion.POLL_MS)
            self._cursor.timeout.connect(self._input.poll)
            self._cursor.start()
        self._probe = QTimer()
        self._probe.setInterval(
            DISCOVERY_POLL_MS if self._watch.available else GEOMETRY_POLL_MS
        )
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
        if self._watch.available:
            self._watch.set_name(name)
        # Answer every push, not only a new name. The page pushes on each
        # channel connect, and a reloaded page holds no geometry — dropping the
        # repeat would leave it placed against the screen until the tracked
        # window next moved.
        self._rect = None
        if name and (self._watch.available or self._xdotool):
            self._poll_geometry(force=True)
            self._probe.start()
        else:
            self._probe.stop()
            self.geometry.emit(0, 0, 0, 0)

    def _on_x_ready(self) -> None:
        """The X connection has something to say. What it is does not matter.

        Looped rather than handled once: asking X where the window is reads the
        connection too, so the answer can arrive with further events behind it
        that leave the queue full and the descriptor quiet.
        """
        while True:
            self._watch.drain()
            self._poll_geometry()
            if not self._watch.pending():
                return

    def _poll_geometry(self, force: bool = False) -> None:
        if self._watch.available:
            changed = self._watch.refresh()
            if not (changed or force):
                return
            rect = self._watch.rect
        else:
            rect = self._window_rect()
            if rect == self._rect and not force:
                return
        self._rect = rect
        surface = self._to_surface(rect) if rect else (0, 0, 0, 0)
        if os.environ.get("LAYER_OVERLAY_DEBUG"):
            x, y, w, h = surface
            print(f"window {self._name!r} {x},{y} {w}x{h}", flush=True)
        self.geometry.emit(*surface)

    def _scale(self) -> float:
        """Device pixels per logical pixel, on the output the surface is on."""
        if self._window is None:
            return 1.0
        return self._window.devicePixelRatio() or 1.0

    def _to_surface(self, rect):
        """The tracked window in the page's own coordinates.

        Two conversions, and the units are the one that bites. X answers in
        device pixels and the page counts in CSS pixels, which are Qt's logical
        ones — the same thing only on an unscaled output. Handed the device
        numbers directly the page puts the overlay 1/scale too far from the
        origin, so the error grows with the distance and the overlay drifts
        away from the window as the window is moved rather than sitting wrong
        by a fixed amount.

        Then the origin. A layer surface covers the output, so that part is
        identity there. A window manager shrinks an X11 surface to the *work
        area* instead, and a panel then offsets it — leaving the page to place
        everything against a screen origin its surface does not start at.

        Fractions of the rectangle — the per-game `--text-*` measurements, and
        both drag offsets — are unaffected: they scale with the width and
        height they are fractions of.
        """
        scale = self._scale()
        x, y, w, h = (round(v / scale) for v in rect)
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
        # Unconditional, not behind LAYER_OVERLAY_DEBUG: "the overlay never
        # appeared" is answered by where the surface went and which output it
        # landed on, and that is the first question every time. One line.
        screen = window.screen()
        print(
            f"{since_start()} surface {window.x()},{window.y()}"
            f" {window.width()}x{window.height()}"
            f" dpr {window.devicePixelRatio()}"
            f" on {screen.name() if screen else '?'}",
            flush=True,
        )
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
        if not self._reported:
            # The end of starting, as far as anyone using it is concerned: the
            # page has loaded, the channel is up and there is something to click.
            # Logged because the surface appearing is *not* that moment and the
            # gap between them is all page load.
            self._reported = True
            print(f"{since_start()} page interactive", flush=True)
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

    @Slot(str)
    def openUrl(self, url: str) -> None:
        """Hand a link to the desktop's browser.

        The surface is the page, so it cannot show one itself. http and https
        only: this slot is reachable by whatever the page loads, and every other
        scheme is a handler on this machine.
        """
        target = QUrl(url)
        if target.scheme() in ("http", "https"):
            QDesktopServices.openUrl(target)

    @Slot()
    def quit(self) -> None:
        """The page asking to be closed.

        Exits [`QUIT_REQUESTED`] rather than 0, so the caller can tell the page
        asking from the process being stopped from outside — a stop is also a
        clean exit, and the two want opposite responses.
        """
        app = QGuiApplication.instance()
        if app is not None:
            app.exit(QUIT_REQUESTED)

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
        # X and the Windows cursor both speak device pixels, and everything above
        # is in the page's logical ones, so the region has to be scaled on the way
        # out. `setMask` is given the logical rectangles because Qt converts them
        # itself.
        explicit = self._input is not None and self._input.available
        scale = self._scale() if explicit else 1.0
        if explicit:
            self._input.apply(int(self._window.winId()), _rects(region, scale))
        else:
            self._window.setMask(region)
        # The region reaches the compositor on the surface's next commit, and a
        # page that has finished painting schedules no further frame — so
        # without this a freshly drawn element can sit on screen taking no
        # clicks until something else happens to repaint.
        self._window.requestUpdate()
        if os.environ.get("LAYER_OVERLAY_DEBUG"):
            rects = " ".join(f"{x},{y} {w}x{h}" for x, y, w, h in _rects(region, scale))
            print(f"mask [{len(self._hits)}] @{scale} {rects}", flush=True)
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
        # Checked before the engine is touched, not only in `_schedule`. Qt still
        # emits `visibleChanged` while it tears the window down on the way out,
        # and by then the engine's C++ object can already be gone — reading
        # `rootObjects()` off it raised a traceback on every quit.
        if self._quitting:
            return
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

    print(f"{since_start()} backend: {BACKEND} ({BACKEND_REASON})", flush=True)

    QtWebEngineQuick.initialize()  # has to precede QGuiApplication
    print(f"{since_start()} web engine initialized", flush=True)
    app = QGuiApplication([sys.argv[0], *qt_args])
    print(f"{since_start()} application up", flush=True)

    # Every output, because the surface covers one of them and a reader with two
    # monitors looking at the wrong one sees nothing at all.
    for screen in app.screens():
        g = screen.geometry()
        print(
            f"{since_start()} screen {screen.name()} {g.x()},{g.y()}"
            f" {g.width()}x{g.height()}"
            f" dpr {screen.devicePixelRatio()}"
            f"{' primary' if screen is app.primaryScreen() else ''}",
            flush=True,
        )
    if not app.screens():
        print(f"{since_start()} no screens - nothing can be drawn on", flush=True)

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

    # Windows has neither signal, so both toggles are the page's own there until
    # something registers a hotkey for them.
    if hasattr(signal, "SIGUSR1"):
        signal.signal(signal.SIGUSR1, lambda *_: overlay.toggle())
        signal.signal(signal.SIGUSR2, lambda *_: overlay.userToggled.emit())
        # Python only runs a signal handler between bytecodes and Qt's event loop
        # is C, so nothing above would ever land without a tick that returns to
        # the interpreter. It has no other work.
        wake = QTimer()
        wake.start(200)
        wake.timeout.connect(lambda: None)

    return app.exec()
