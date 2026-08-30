"""Whether the window being read is the one in use.

Windows-only: `ctypes.wintypes` does not import anywhere else. There is no
counterpart under X11 or Wayland — see the README.

The surface is above every window, which is right while the game is being read
and wrong the moment anything else is: a browser or an editor brought to the
front is drawn *under* it and the line sits over unrelated text. So the answer is
reported to the page, which draws the line or leaves it out. Reported rather than
enforced here, because what the surface is *for* is the page's to decide — it
keeps its controls reachable and drops only the line.

The rule: the game's process in front counts, this process counts too, and no
window tracked counts — a game that has not started yet must still leave the
surface usable.

Counting this process is what keeps the answer from oscillating. The surface
taking focus reads as "the game is no longer in front", which would drop the line
under the reader's hands as they open a panel.
"""

import ctypes
import os
import sys
from ctypes import wintypes


class Focus:
    """Reports which side of that rule the foreground window is on.

    No timer and no hook of its own: [`poll`] is driven from the caller's event
    loop beside [`wininput.InputRegion.poll`], which already runs at the rate a
    cursor needs. A window switch matters at the rate a person can make one, so
    that is far more often than enough, and a `SetWinEventHook` here would only
    be a second thing to keep alive.
    """

    def __init__(self) -> None:
        self._seen = None
        self._in_front = True
        self._pid = os.getpid()
        user32 = ctypes.WinDLL("user32", use_last_error=True)
        user32.GetForegroundWindow.restype = wintypes.HWND
        user32.GetWindowThreadProcessId.restype = wintypes.DWORD
        user32.GetWindowThreadProcessId.argtypes = [
            wintypes.HWND, ctypes.POINTER(wintypes.DWORD)
        ]
        user32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
        self._user32 = user32

    @property
    def in_front(self) -> bool:
        """The last answer, for a caller that needs it without a change."""
        return self._in_front

    def repeat(self) -> None:
        """Answer the next [`poll`] even if nothing has changed.

        The page pushes on each channel connect, and a reloaded page holds no
        answer at all.
        """
        self._seen = None

    def poll(self, tracked: int) -> bool | None:
        """The answer, or None while it is the same as last time.

        `tracked` is the game's window, 0 for none.
        """
        foreground = self._user32.GetForegroundWindow()
        # Null between windows, and while a UAC prompt owns the secure desktop.
        if not foreground:
            return None
        if (foreground, tracked) == self._seen:
            return None
        self._seen = (foreground, tracked)
        if not tracked:
            self._in_front = True
            why = "no window tracked"
        else:
            self._in_front = self._pid_of(foreground) in (
                self._pid, self._pid_of(tracked)
            )
            why = f"{self._title(foreground)!r} in front"
        print(
            f"focus: {'reading' if self._in_front else 'elsewhere'}, {why}",
            flush=True,
        )
        return self._in_front

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
