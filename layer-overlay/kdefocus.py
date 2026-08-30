"""Whether the game is the window in use, asked of KWin.

The Linux counterpart to [`winfocus`], and the same rule: the surface's line
belongs on screen while the game is being read and is in the way the moment
anything else is. What differs is where the answer comes from and what is done
with it.

**Why KWin and not Wayland.** There is no protocol for this. A client cannot ask
what is focused — `wlr-foreign-toplevel-management`, `ext-foreign-toplevel-list`
and `plasma-window-management` all carry the answer and KWin advertises none of
them to an unprivileged client. So this is KDE-only, by the compositor rather
than by choice, and every other desktop gets [`gate`] returning None.

What makes KWin worth the dependency is that it is the compositor for *both*
sides. The README's objection to a focus gate here — a game in XWayland, a
native layer surface, and no question that answers for both — is an objection to
asking the display server, which under a Wayland session is two display servers.
KWin is one, and it answers for the game whichever it is.

**How the answer gets out.** A KWin script, loaded over D-Bus at startup, which
connects to `workspace.windowActivated` and calls back into this process. Twenty
lines of JavaScript that KWin runs in its own address space; the interesting
part of this module is its lifecycle, not its logic.

**What is done with it.** Nothing, here. The answer is reported and the page
acts on it — `inFront` on the channel, folded into the same state the bar's hide
button writes, so the line goes and the controls stay. That is [`winfocus`]'s
bargain too, and this module holds up the same end of it.

What it does *not* have is the other half of `_windows_tick`. A window coming to
the front there is also the moment the topmost band may have moved, so the same
event drives a re-raise. A layer surface is above by protocol and there is
nothing to defend, which is why this reports and stops.

`LAYER_OVERLAY_FOCUS_GATE=0` turns it off, the same knob as on Windows.
"""

import os
import tempfile
from pathlib import Path

from PySide6.QtCore import ClassInfo, QObject, QTimer, Signal, Slot
from PySide6.QtDBus import QDBusConnection, QDBusInterface, QDBusServiceWatcher

KWIN = "org.kde.KWin"
SCRIPTING = "/Scripting"
SCRIPTING_IFACE = "org.kde.kwin.Scripting"

#: The name this process answers to on the bus, and the object on it. Both carry
#: the pid: two overlays on one session are two scripts in one KWin, and a name
#: they shared would have the second one's script reporting to the first.
SERVICE = "org.layeroverlay.focus.p{pid}"
PATH = "/focus"
IFACE = "org.layeroverlay.Focus"

#: How long something else has to be in front before the line goes. A KDE
#: session activates windows the reader never chose — the panel takes focus and
#: hands it straight back, a notification comes and goes — and each of those is
#: a game→plasmashell→game round trip in under a frame. Coming *back* is not
#: delayed: the line reappearing late is the reader waiting on their own click.
SETTLE_MS = 400

#: What KWin runs. `activeWindow` is read once at load as well as connected to:
#: the script starts long after the session does, and without it nothing is
#: known until the next time the reader switches windows.
SCRIPT = """
function report(w) {{
    callDBus("{service}", "{path}", "{iface}", "activated",
             w ? "" + w.caption : "", w ? w.pid : 0);
}}
workspace.windowActivated.connect(report);
report(workspace.activeWindow);
"""


def gate(on_change):
    """The gate, or None where there is nothing to ask or nobody to ask it.

    `on_change` takes one bool: whether what is in use is the game or this
    program. Called only when the answer changes.
    """
    if os.environ.get("LAYER_OVERLAY_FOCUS_GATE", "1").strip() == "0":
        print("focus gate: off, the line stays over every window", flush=True)
        return None
    bus = QDBusConnection.sessionBus()
    if not bus.isConnected():
        print("focus gate: off, no session bus", flush=True)
        return None
    if not bus.interface().isServiceRegistered(KWIN).value():
        print("focus gate: off, KWin is not on the bus", flush=True)
        return None
    gate_ = FocusGate(bus, on_change)
    return gate_ if gate_.available else None


@ClassInfo({"D-Bus Interface": IFACE})
class FocusGate(QObject):
    """Follows KWin's active window and judges it against the tracked one.

    The judgement is [`winfocus.FocusGate`]'s, for the reasons given there:

    - **This process counts as the game.** Written against focus alone the rule
      oscillates — the overlay takes focus, which reads as the game no longer
      being in front, so the line hides, which hands focus back to the game,
      which shows it. A layer surface is not a window KWin scripts see, so it
      should never be reported here at all; this makes the loop impossible where
      it is.
    - **Nothing tracked is not the same as something else in front.** A game
      that has quit or has not started yet leaves the line where it was, rather
      than hiding it against a window that does not exist.

    The game is *found* by title substring — the same string [`xwatch`] finds
    the window with, so both halves of the overlay follow one setting — and then
    followed by the pid that arrived beside the caption that matched. See
    [`_judge`] for why it takes both.
    """

    #: Whether the game or this program is what is in use. Wired to the
    #: overlay's `inFront`, which is what the page hears.
    changed = Signal(bool)

    def __init__(self, bus, on_change) -> None:
        super().__init__()
        self._bus = bus
        self._name = ""
        #: The active window as `caption, pid`, or None until KWin has said.
        #: Not the same as no window: nothing is judged before the first answer,
        #: or the line hides for as long as the script takes to load.
        self._active = None
        #: Which process the tracked window turned out to belong to, learned
        #: from the caption that matched. See [`_judge`].
        self._game = 0
        self._ours = True
        self._want = (True, "")
        self._settle = QTimer(self)
        self._settle.setSingleShot(True)
        self._settle.setInterval(SETTLE_MS)
        self._settle.timeout.connect(self._apply)
        self._pid = os.getpid()
        self._script = None
        self._plugin = f"layer-overlay-focus-{self._pid}"
        self.changed.connect(on_change)
        self._service = SERVICE.format(pid=self._pid)
        #: False where the bus name could not be taken, which is the one failure
        #: that leaves the script with nowhere to report to. [`gate`] answers
        #: None for it, so the caller has one way to mean "no gate".
        self.available = bus.registerService(self._service)
        if not self.available:
            print(f"focus gate: off, {self._service} is taken", flush=True)
            return
        bus.registerObject(PATH, self, QDBusConnection.ExportAllSlots)
        # KWin restarting drops every script it had loaded, and takes the
        # overlay's with them. Watched rather than assumed permanent because a
        # compositor restart is a thing a reader does — it is what a Plasma
        # crash or a `kwin_wayland --replace` looks like from here.
        self._watcher = QDBusServiceWatcher(
            KWIN, bus, QDBusServiceWatcher.WatchForOwnerChange, self
        )
        self._watcher.serviceOwnerChanged.connect(self._owner_changed)
        self._load()

    def set_name(self, name: str) -> None:
        """Which window is the game, by title substring. `""` for none."""
        name = (name or "").strip()
        if name == self._name:
            return
        self._name = name
        self._game = 0
        self._judge()

    def close(self) -> None:
        """Take the script back out of KWin.

        KWin outlives this process, and a script left loaded goes on calling a
        bus name that has gone — which is a warning in the journal per window
        switch, forever.
        """
        if self._script is not None:
            self._call("unloadScript", self._plugin)
            self._script.unlink(missing_ok=True)
            self._script = None

    @Slot(str, int)
    def activated(self, caption: str, pid: int) -> None:
        """KWin's active window, called from the script.

        A window with neither a caption nor a pid is KWin saying there is no
        active window — between two windows, or on the desktop. That is not a
        window to judge against, and treating it as one flickers the line
        through every switch.
        """
        if os.environ.get("LAYER_OVERLAY_DEBUG"):
            print(f"focus: active {caption!r} pid {pid}", flush=True)
        if not caption and not pid:
            return
        self._active = (caption, pid)
        self._judge()

    def _judge(self) -> None:
        """Whether the game or this program is what is in use.

        The game is *found* by caption and then followed by pid. Its own menus
        and dialogs carry captions of their own — a Kirikiri game's right-click
        menu comes through as `kcMenuWindow` — and every one of them would read
        as leaving the game if the caption were the whole test. Which is what
        Windows gets for free from `GetWindowThreadProcessId` and what this has
        to learn: the pid arrives beside the caption that matched.
        """
        if self._active is None:
            return
        caption, pid = self._active
        if not self._name:
            ours, why = True, "no window tracked"
        elif pid and pid == self._pid:
            ours, why = True, "the overlay itself"
        elif self._name.casefold() in caption.casefold():
            self._game = pid
            ours, why = True, f"{caption!r} in front"
        elif pid and pid == self._game:
            ours, why = True, f"{caption!r}, the game's own window"
        else:
            ours, why = False, f"{caption!r} in front"
        self._want = (ours, why)
        # Hiding waits out a session that hands focus around on its own; showing
        # does not, and cancels a hide that has not landed.
        if ours:
            self._settle.stop()
            self._apply()
        elif self._ours and not self._settle.isActive():
            self._settle.start()

    def _apply(self) -> None:
        ours, why = self._want
        if ours == self._ours:
            return
        self._ours = ours
        # Unconditional rather than behind LAYER_OVERLAY_DEBUG: "the line
        # disappeared" is answered by what was in front when it did, and a line
        # per window switch is nothing.
        print(f"focus: line {'shown' if ours else 'hidden'}, {why}", flush=True)
        self.changed.emit(ours)

    def _owner_changed(self, _service, _old, new_owner) -> None:
        if new_owner:
            self._script = None
            self._load()

    def _load(self) -> None:
        """Put the script into KWin. Idempotent: unload first, since a plugin
        name already loaded is a load KWin answers with the id it already had
        and no new script."""
        self._call("unloadScript", self._plugin)
        path = Path(tempfile.mkdtemp(prefix="layer-overlay-")) / "focus.js"
        path.write_text(
            SCRIPT.format(service=self._service, path=PATH, iface=IFACE)
        )
        reply = self._call("loadScript", str(path), self._plugin)
        if reply is None or reply.errorMessage():
            path.unlink(missing_ok=True)
            print("focus gate: off, KWin would not load the script", flush=True)
            return
        self._script = path
        # Loaded is not running: KWin starts scripts as a batch, and one loaded
        # after the session came up has missed the batch it would have been in.
        self._call("start")

    def _call(self, method: str, *args):
        iface = QDBusInterface(KWIN, SCRIPTING, SCRIPTING_IFACE, self._bus)
        if not iface.isValid():
            return None
        return iface.call(method, *args)
