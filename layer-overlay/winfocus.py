"""Whether the surface belongs on screen at all: is the game the window in use.

Windows-only: `ctypes.wintypes` does not import anywhere else. There is no
counterpart under X11 or Wayland — see the README.

A surface above every window is right while the game is being read and wrong the
moment anything else is. It is topmost, so a browser or an editor brought to the
front is drawn *under* it and the strip sits over unrelated text.

The rule: show the surface while the foreground window belongs to the game's
process or to this one, hide it otherwise, and never hide while no window is
being tracked — a game that has not started yet must still leave the ✕
reachable.

Counting this process as the game's is what keeps the rule from oscillating. The
surface taking focus reads as "the game is no longer in front", so it hides,
which hands focus back to the game, which shows it again. `WS_EX_NOACTIVATE`
already means it should never become the foreground window here, and this makes
the loop impossible even where it does.

Shown and hidden with `ShowWindow` rather than through Qt. `QWindow.setVisible`
is what a compositor closing the surface looks like to [`layer_overlay.Surface`],
which answers it by building a new window; a native hide leaves Qt's idea of the
window alone.

`LAYER_OVERLAY_FOCUS_GATE=0` turns it off, for a reader who wants the surface on
top of everything regardless.
"""

import ctypes
import os
import sys
from ctypes import wintypes

SW_HIDE = 0
SW_SHOWNOACTIVATE = 4


def gate():
    """The gate, or None where the reader has turned it off."""
    if os.environ.get("LAYER_OVERLAY_FOCUS_GATE", "1").strip() == "0":
        print("focus gate: off, the surface stays over every window", flush=True)
        return None
    return FocusGate()


class FocusGate:
    """Hides the surface while something other than the game is being used.

    No timer and no hook of its own: [`poll`] is driven from the caller's event
    loop beside [`wininput.InputRegion.poll`], which already runs at the rate a
    cursor needs. A window switch matters at the rate a person can make one, so
    that is far more often than enough, and a `SetWinEventHook` here would only
    be a second thing to keep alive.
    """

    def __init__(self) -> None:
        self._surface = 0
        self._visible = True
        #: The last `(foreground, tracked)` judged, so the process lookups run
        #: on a change rather than on every tick.
        self._seen = None
        self._pid = os.getpid()
        user32 = ctypes.WinDLL("user32", use_last_error=True)
        user32.GetForegroundWindow.restype = wintypes.HWND
        user32.GetWindowThreadProcessId.restype = wintypes.DWORD
        user32.GetWindowThreadProcessId.argtypes = [
            wintypes.HWND, ctypes.POINTER(wintypes.DWORD)
        ]
        user32.ShowWindow.restype = wintypes.BOOL
        user32.ShowWindow.argtypes = [wintypes.HWND, ctypes.c_int]
        user32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
        self._user32 = user32

    def poll(self, surface: int, tracked: int) -> None:
        """Put the surface in the state the foreground window calls for.

        `surface` is this program's window, `tracked` the game's, 0 for none.
        """
        if not surface:
            return
        if surface != self._surface:
            # A rebuilt window is a new handle, shown by Qt and knowing nothing
            # of what the old one had been put into.
            self._surface = surface
            self._visible = True
            self._seen = None
        foreground = self._user32.GetForegroundWindow()
        # Null while activation moves between windows, and for as long as a UAC
        # prompt owns the secure desktop. Neither is a window to judge against,
        # and treating it as one flickers the surface through every switch.
        if not foreground:
            return
        if (foreground, tracked) == self._seen:
            return
        self._seen = (foreground, tracked)
        if not tracked:
            self._show(True, "no window tracked")
        else:
            here = self._pid_of(foreground) in (self._pid, self._pid_of(tracked))
            self._show(here, f"{self._title(foreground)!r} in front")

    def _pid_of(self, window: int) -> int:
        """Which process the window belongs to, so the game's own dialogs — a
        config window, a save prompt — count as the game rather than as leaving
        it."""
        pid = wintypes.DWORD()
        self._user32.GetWindowThreadProcessId(window, ctypes.byref(pid))
        return pid.value

    def _title(self, window: int) -> str:
        text = ctypes.create_unicode_buffer(256)
        self._user32.GetWindowTextW(window, text, len(text))
        encoding = getattr(sys.stdout, "encoding", None) or "ascii"
        return text.value.encode(encoding, "replace").decode(encoding, "replace")

    def _show(self, visible: bool, why: str) -> None:
        if visible == self._visible:
            return
        self._visible = visible
        self._user32.ShowWindow(
            self._surface, SW_SHOWNOACTIVATE if visible else SW_HIDE
        )
        # Unconditional rather than behind LAYER_OVERLAY_DEBUG: "the overlay
        # disappeared" is answered by what was in front when it did, and a line
        # per window switch is nothing.
        print(f"focus: {'shown' if visible else 'hidden'}, {why}", flush=True)
