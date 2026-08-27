"""What takes clicks on Windows, which has no input region to set.

Windows-only: `ctypes.wintypes` does not import anywhere else. [`layer_overlay`]
imports this or [`xshape`] by backend, and the two set a region the same way.

X11 has `ShapeInput` and Wayland has `wl_surface.set_input_region`: both let a
window say "clicks land in these boxes and pass through everywhere else", once,
and then forget about it. Windows has nothing of the kind. `SetWindowRgn` is the
near miss, and taking it costs the whole overlay — a window region clips
*painting* as well as input, so the surface would be visible only where it is
clickable, and the page draws text in places that take no clicks at all.

`WS_EX_TRANSPARENT` is the only real lever, and it is per-window rather than
per-region: the whole window either takes clicks or passes them all through. So
the region is emulated. The cursor is watched, tested against the boxes the page
reported, and the bit is set or cleared as it crosses the boundary — two states,
switched on a change, rather than a decision per click.

The consequence, and it is the only place this backend is weaker than the other
two: what takes clicks follows the *cursor*, not the click. A click delivered
somewhere the cursor has not been — a synthetic one, or a tablet tapping a fresh
position — arrives before the boundary is noticed. Nothing a mouse does can
produce that, since a click is always preceded by the move that got there.

Interface is [`xshape.InputRegion`]'s on purpose, so the caller sets a region
the same way on both. No timer of its own: [`poll`] is driven from the caller's
event loop, beside the other timers.
"""

import ctypes
from ctypes import wintypes

GWL_EXSTYLE = -20
WS_EX_LAYERED = 0x00080000
WS_EX_TRANSPARENT = 0x00000020
WS_EX_TOPMOST = 0x00000008
WS_EX_NOACTIVATE = 0x08000000

HWND_TOPMOST = -1
SWP_NOSIZE = 0x0001
SWP_NOMOVE = 0x0002
SWP_NOACTIVATE = 0x0010

#: How often the caller should [`poll`]. A click is preceded by the move that
#: got the cursor there, so the boundary only has to be noticed within the gap
#: between a mouse arriving somewhere and the button going down.
POLL_MS = 16


class InputRegion:
    """The boxes that take clicks, kept as a `WS_EX_TRANSPARENT` state.

    `rects` are in the same units [`xshape`] takes them in — device pixels,
    relative to the surface's own origin — and the cursor is converted into
    those before it is tested, so nothing here needs to know the scale.
    """

    def __init__(self) -> None:
        self._hwnd = 0
        self._rects = []
        self._click_through = None
        user32 = ctypes.WinDLL("user32", use_last_error=True)
        user32.GetWindowLongPtrW.restype = ctypes.c_longlong
        user32.GetWindowLongPtrW.argtypes = [wintypes.HWND, ctypes.c_int]
        user32.SetWindowLongPtrW.restype = ctypes.c_longlong
        user32.SetWindowLongPtrW.argtypes = [
            wintypes.HWND, ctypes.c_int, ctypes.c_longlong,
        ]
        user32.GetCursorPos.argtypes = [ctypes.POINTER(wintypes.POINT)]
        user32.GetWindowRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT)]
        user32.GetForegroundWindow.restype = wintypes.HWND
        user32.SetWindowPos.argtypes = [
            wintypes.HWND, wintypes.HWND, ctypes.c_int, ctypes.c_int,
            ctypes.c_int, ctypes.c_int, ctypes.c_uint,
        ]
        self._user32 = user32
        self._foreground = None

    @property
    def available(self) -> bool:
        """True: user32 is part of the platform. The X11 region can genuinely
        fail to open a display, and the caller asks both the same question."""
        return True

    def apply(self, window_id: int, rects) -> bool:
        """`rects` is a sequence of `(x, y, w, h)`. Empty means nothing clickable."""
        if not window_id:
            return False
        if window_id != self._hwnd:
            self._hwnd = window_id
            # A rebuilt window is a new hwnd with a fresh style, so the state
            # this class thinks it set is not on it.
            self._click_through = None
            self._set_base_style()
        self._rects = [
            (int(x), int(y), max(int(w), 1), max(int(h), 1)) for x, y, w, h in rects
        ]
        self.poll()
        return True

    def _set_base_style(self) -> None:
        """Layered, topmost and never focused, for as long as the window lives.

        Qt asks for the last two through window flags as well. Set here anyway
        because this is the one place that reads and writes the whole style
        word: taking Qt's value and putting back a value that dropped bits Qt
        had set is how a topmost overlay quietly stops being topmost.

        `WS_EX_LAYERED` is the one that is this module's own. It is what makes
        the surface composite with per-pixel alpha against whatever is behind
        it, rather than against black.
        """
        ex = self._user32.GetWindowLongPtrW(self._hwnd, GWL_EXSTYLE)
        ex |= WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE
        self._user32.SetWindowLongPtrW(self._hwnd, GWL_EXSTYLE, ex)
        self._raise()

    def _raise(self) -> None:
        """Put the window in the topmost band.

        `WS_EX_TOPMOST` in the style word does not do this. The bit says the
        window is topmost and `SetWindowPos` is what actually puts it there, so
        setting the style alone leaves a window that reads as topmost, is not,
        and is quietly covered by the next thing the user opens.

        `SWP_NOACTIVATE` because the surface must never take focus: raising it
        over a game by stealing the game's focus would pause the game.
        """
        self._user32.SetWindowPos(
            self._hwnd, HWND_TOPMOST, 0, 0, 0, 0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )

    def _set_click_through(self, on: bool) -> None:
        if on == self._click_through:
            return
        self._click_through = on
        ex = self._user32.GetWindowLongPtrW(self._hwnd, GWL_EXSTYLE)
        ex = ex | WS_EX_TRANSPARENT if on else ex & ~WS_EX_TRANSPARENT
        self._user32.SetWindowLongPtrW(self._hwnd, GWL_EXSTYLE, ex)

    def poll(self) -> None:
        """Put the window in the state the cursor's position calls for.

        Also the place the topmost band is defended from. A game that asserts
        topmost on focus pushes the overlay under itself, and the only signal
        that has happened is the focus change — so the foreground window is
        watched and the surface re-raised whenever it moves to anything else.
        Cheaper than a hook, and this already runs at the rate a cursor needs.
        """
        if not self._hwnd:
            return
        foreground = self._user32.GetForegroundWindow()
        if foreground != self._foreground:
            self._foreground = foreground
            if foreground != self._hwnd:
                self._raise()
        cursor = wintypes.POINT()
        frame = wintypes.RECT()
        if not self._user32.GetCursorPos(ctypes.byref(cursor)):
            return
        if not self._user32.GetWindowRect(self._hwnd, ctypes.byref(frame)):
            return
        x = cursor.x - frame.left
        y = cursor.y - frame.top
        inside = any(
            rx <= x < rx + rw and ry <= y < ry + rh for rx, ry, rw, rh in self._rects
        )
        self._set_click_through(not inside)
