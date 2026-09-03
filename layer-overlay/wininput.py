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

The wheel is the one input that no region can route, and [`WheelGuard`] is
here for it. Windows delivers `WM_MOUSEWHEEL` to the *focused* window rather
than the hovered one, and this surface never takes focus, so every notch reaches
the game underneath — including the ones aimed at a popup the page has drawn,
and the ones aimed at another window entirely while the game holds focus. A
low-level hook is the only place that can be answered.

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

TOKEN_QUERY = 0x0008
TOKEN_INTEGRITY_LEVEL = 25
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
ERROR_INSUFFICIENT_BUFFER = 122

WH_MOUSE_LL = 14
HC_ACTION = 0
WM_MOUSEWHEEL = 0x020A
WM_MOUSEHWHEEL = 0x020E

VK_LBUTTON = 0x01
VK_RBUTTON = 0x02
KEY_DOWN = 0x8000

#: How often the caller should [`poll`]. A click is preceded by the move that
#: got the cursor there, so the boundary only has to be noticed within the gap
#: between a mouse arriving somewhere and the button going down.
POLL_MS = 16

#: How long a raise keeps re-asserting itself. An upscaler asserts its own
#: topmost as the game is focused and the last raise wins, so one raise is a
#: race against it and a raise per tick over this long is not.
SETTLE_MS = 500

#: How many polls an escort has to land the cursor. An upscaler translates the
#: position it is handed once more on the way to letting go, so the first move
#: overshoots and it is the correction after it that arrives.
ESCORT_POLLS = 5

#: How long after a move before another may start. Nothing observed re-fences a
#: cursor that has been taken out, but an upscaler that did would otherwise be
#: argued with at the poll rate, and a cursor shaking in place is worse than one
#: that cannot reach the line.
ESCORT_REST_POLLS = 30


class SID_AND_ATTRIBUTES(ctypes.Structure):
    _fields_ = [("Sid", ctypes.c_void_p), ("Attributes", wintypes.DWORD)]


class TOKEN_MANDATORY_LABEL(ctypes.Structure):
    _fields_ = [("Label", SID_AND_ATTRIBUTES)]


class MSLLHOOKSTRUCT(ctypes.Structure):
    _fields_ = [
        ("pt", wintypes.POINT),
        ("mouseData", wintypes.DWORD),
        ("flags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ctypes.c_void_p),
    ]


_HOOKPROC = ctypes.WINFUNCTYPE(
    ctypes.c_ssize_t, ctypes.c_int, wintypes.WPARAM, wintypes.LPARAM
)


class InputRegion:
    """The boxes that take clicks, kept as a `WS_EX_TRANSPARENT` state.

    `rects` are in the same units [`xshape`] takes them in — device pixels,
    relative to the surface's own origin — and the cursor is converted into
    those before it is tested, so nothing here needs to know the scale.
    """

    on_click_outside = None

    def __init__(self) -> None:
        self._hwnd = 0
        self._rects = []
        self._click_through = None
        self._buttons_down = False
        self._keyboard = False
        self._settle = 0
        self._escort_to = None
        self._escort_left = 0
        self._escort_rest = 0
        user32 = ctypes.WinDLL("user32", use_last_error=True)
        user32.GetWindowLongPtrW.restype = ctypes.c_longlong
        user32.GetWindowLongPtrW.argtypes = [wintypes.HWND, ctypes.c_int]
        user32.SetWindowLongPtrW.restype = ctypes.c_longlong
        user32.SetWindowLongPtrW.argtypes = [
            wintypes.HWND, ctypes.c_int, ctypes.c_longlong,
        ]
        user32.GetCursorPos.argtypes = [ctypes.POINTER(wintypes.POINT)]
        user32.SetCursorPos.argtypes = [ctypes.c_int, ctypes.c_int]
        user32.ClipCursor.argtypes = [ctypes.POINTER(wintypes.RECT)]
        user32.GetWindowRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT)]
        user32.GetAsyncKeyState.restype = ctypes.c_short
        user32.GetAsyncKeyState.argtypes = [ctypes.c_int]
        user32.SetForegroundWindow.restype = wintypes.BOOL
        user32.SetForegroundWindow.argtypes = [wintypes.HWND]
        user32.SetWindowPos.argtypes = [
            wintypes.HWND, wintypes.HWND, ctypes.c_int, ctypes.c_int,
            ctypes.c_int, ctypes.c_int, ctypes.c_uint,
        ]
        self._user32 = user32

    @property
    def available(self) -> bool:
        """True: user32 is part of the platform. The X11 region can genuinely
        fail to open a display, and the caller asks both the same question."""
        return True

    @property
    def hwnd(self) -> int:
        return self._hwnd

    def covers_point(self, screen_x: int, screen_y: int) -> bool:
        """Whether a screen point is on something the page has drawn."""
        if not self._hwnd:
            return False
        frame = wintypes.RECT()
        if not self._user32.GetWindowRect(self._hwnd, ctypes.byref(frame)):
            return False
        return self._covered(screen_x - frame.left, screen_y - frame.top)

    def apply(self, window_id: int, rects) -> bool:
        """`rects` is a sequence of `(x, y, w, h)`. Empty means nothing clickable."""
        if not window_id:
            return False
        if window_id != self._hwnd:
            self._hwnd = window_id
            # A rebuilt window is a new hwnd with a fresh style, so the state
            # this class thinks it set is not on it.
            self._click_through = None
            self._keyboard = False
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

    def raise_topmost(self) -> None:
        """Put the surface back at the top of the topmost band, and keep it there.

        Both of the things that push it under happen on an event the caller
        already has: a game that asserts topmost as it takes focus, and an
        upscaler that draws into a topmost window of its own as it starts
        scaling. What neither gives is a moment when the raise can be made
        *last*, so it is made repeatedly for [`SETTLE_MS`] — after which it
        holds, since nothing here re-asserts a band it has been taken out of.
        """
        if self._hwnd:
            self._settle = SETTLE_MS // POLL_MS
            self._raise()

    def set_keyboard(self, want: bool, restore_to: int = 0) -> None:
        if not self._hwnd or want == self._keyboard:
            return
        self._keyboard = want
        ex = self._user32.GetWindowLongPtrW(self._hwnd, GWL_EXSTYLE)
        ex = ex & ~WS_EX_NOACTIVATE if want else ex | WS_EX_NOACTIVATE
        self._user32.SetWindowLongPtrW(self._hwnd, GWL_EXSTYLE, ex)
        if not want and restore_to:
            self._user32.SetForegroundWindow(restore_to)

    def _set_click_through(self, on: bool) -> None:
        if on == self._click_through:
            return
        self._click_through = on
        ex = self._user32.GetWindowLongPtrW(self._hwnd, GWL_EXSTYLE)
        ex = ex | WS_EX_TRANSPARENT if on else ex & ~WS_EX_TRANSPARENT
        self._user32.SetWindowLongPtrW(self._hwnd, GWL_EXSTYLE, ex)

    def poll(self, aim=None) -> None:
        """Put the window in the state the cursor's position calls for, and carry
        on with whatever [`raise_topmost`] has left to do.

        `aim` answers where the reader is pointing when that is not where the
        cursor is — [`winwatch.WindowWatch.aim`], or None where nothing can move
        the two apart.
        """
        if not self._hwnd:
            return
        if self._settle:
            self._settle -= 1
            self._raise()
        cursor = wintypes.POINT()
        frame = wintypes.RECT()
        if not self._user32.GetCursorPos(ctypes.byref(cursor)):
            return
        if not self._user32.GetWindowRect(self._hwnd, ctypes.byref(frame)):
            return
        inside = self._covered(cursor.x - frame.left, cursor.y - frame.top)
        if inside:
            self._escort_to = None
        else:
            inside = self._escort(cursor, frame, aim)
        self._set_click_through(not inside)
        self._note_click(inside)

    def _covered(self, x: int, y: int) -> bool:
        """Whether a point in the surface's own coordinates takes clicks."""
        return any(
            rx <= x < rx + rw and ry <= y < ry + rh for rx, ry, rw, rh in self._rects
        )

    def _escort(self, cursor, frame, aim) -> bool:
        """Bring the cursor to what the reader is pointing at.

        An upscaler keeps the cursor inside the window it is scaling, so a reader
        aiming at the picture cannot put it on anything the page has drawn out
        there: the press would land where the cursor really is, back in the small
        original, and reach the game instead. Once the cursor is out of that
        fence the upscaler stops asserting one, so this is a move rather than a
        fight — but the move needs repeating, because the position it is handed
        is translated once more on the way out.

        Never while a button is down: that is a drag or a click on the game, and
        taking the cursor out from under one is not what was asked for.
        """
        if self._escort_to is not None:
            # Mid-move, where the cursor is says nothing about where the reader
            # is pointing: it is wherever the last correction was translated to.
            if (cursor.x, cursor.y) == self._escort_to or not self._escort_left:
                self._escort_to = None
                self._escort_rest = ESCORT_REST_POLLS
                return True
            self._escort_left -= 1
            self._move_to(self._escort_to)
            return True
        if self._escort_rest:
            self._escort_rest -= 1
            return False
        if aim is None or self._buttons_down:
            return False
        target = aim(cursor.x, cursor.y)
        if target is None or not self._covered(
            target[0] - frame.left, target[1] - frame.top
        ):
            return False
        self._escort_to = target
        self._escort_left = ESCORT_POLLS
        self._move_to(target)
        return True

    def _move_to(self, target) -> None:
        """Put the cursor somewhere, past whatever is fencing it in."""
        self._user32.ClipCursor(None)
        self._user32.SetCursorPos(*target)

    def _note_click(self, inside: bool) -> None:
        down = any(
            self._user32.GetAsyncKeyState(vk) & KEY_DOWN
            for vk in (VK_LBUTTON, VK_RBUTTON)
        )
        pressed, self._buttons_down = down and not self._buttons_down, down
        if pressed and not inside and self.on_click_outside is not None:
            self.on_click_outside()


class WheelGuard:
    """Where a wheel notch goes, since Windows sends it to the focused window.

    `WM_MOUSEWHEEL` is delivered to whatever holds focus, not to what the cursor
    is over. This surface is `WS_EX_NOACTIVATE` and must stay that way — taking
    the game's focus to raise a page over it would pause the game — so with the
    game focused every notch is the game's: one aimed at a popup the page has
    drawn scrolls the game as well, and one aimed at another window while the
    game is still focused scrolls the game instead of that window.

    A `WH_MOUSE_LL` hook is the only place that sees a notch before the focused
    window does. Three answers, by where the cursor is:

    - on something the page has drawn — swallow it and post it to the surface,
      so the popup scrolls and the game sees nothing
    - outside the tracked window — swallow it. It cannot reach the window it was
      aimed at anyway, since that window is not the focused one; passing it on
      only lets it advance the game
    - on the tracked window — pass it through, which is the only case where the
      game is what was aimed at

    Nothing tracked means no rectangle to be outside of, so everything the page
    has not drawn on passes through.

    The hook runs on the thread that installed it, during that thread's message
    loop — the caller's Qt loop — so there is no thread of its own and nothing
    to lock. It sees every mouse *move* as well, which is why the path to
    `CallNextHookEx` for anything that is not a wheel is the first branch.

    A posted message is not hardware input and does not come back round through
    the hook, so the forward cannot loop.
    """

    def __init__(self, region: "InputRegion") -> None:
        self._region = region
        self._tracked = None
        self._hook = 0
        user32 = region._user32
        user32.SetWindowsHookExW.restype = wintypes.HHOOK
        user32.SetWindowsHookExW.argtypes = [
            ctypes.c_int, ctypes.c_void_p, wintypes.HINSTANCE, wintypes.DWORD,
        ]
        user32.UnhookWindowsHookEx.argtypes = [wintypes.HHOOK]
        user32.CallNextHookEx.restype = ctypes.c_ssize_t
        user32.CallNextHookEx.argtypes = [
            wintypes.HHOOK, ctypes.c_int, wintypes.WPARAM, wintypes.LPARAM,
        ]
        user32.PostMessageW.argtypes = [
            wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM,
        ]
        self._user32 = user32
        user32.GetWindowThreadProcessId.argtypes = [
            wintypes.HWND, ctypes.POINTER(wintypes.DWORD),
        ]
        self._advapi32 = _integrity_api()
        self._blocked = None
        self._asked_for = 0
        # Held on the instance because ctypes keeps no reference of its own: a
        # callback collected while the hook is installed is a crash in user32.
        self._proc = _HOOKPROC(self._on_mouse)

    def install(self) -> bool:
        if self._hook:
            return True
        self._hook = self._user32.SetWindowsHookExW(
            WH_MOUSE_LL, ctypes.cast(self._proc, ctypes.c_void_p), None, 0
        )
        return bool(self._hook)

    def close(self) -> None:
        if self._hook:
            self._user32.UnhookWindowsHookEx(self._hook)
            self._hook = 0

    def set_tracked_rect(self, rect) -> None:
        """Where the tracked window is, as `(x, y, w, h)` in screen pixels."""
        self._tracked = rect

    def blocked_by(self, hwnd: int):
        """Whether the hook is silently skipped over this window; None if unknown.

        Windows does not call a low-level hook installed by a process at a lower
        integrity level than the foreground window's. There is no error and no
        callback — the notch simply goes on to the game — so nothing inside the
        hook can tell. A VN started as administrator, or an upscaler that starts
        one, is exactly that case.

        Comparing the two levels is the only way to know, and the reader is who
        has to act on it: the same surface started as administrator installs a
        hook that works. `uiAccess` is the documented alternative and needs a
        signed binary under Program Files.

        Cached per window, because the answer cannot change while a process
        lives and the caller asks on a timer.
        """
        if not hwnd or self._advapi32 is None:
            return None
        if hwnd == self._asked_for:
            return self._blocked
        self._asked_for = hwnd
        self._blocked = self._compare(hwnd)
        return self._blocked

    def _compare(self, hwnd: int):
        kernel32, _ = self._advapi32
        pid = wintypes.DWORD()
        self._user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        if not pid.value:
            return None
        theirs_handle = kernel32.OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION, False, pid.value
        )
        if not theirs_handle:
            return None
        try:
            theirs = _integrity_level(self._advapi32, theirs_handle)
        finally:
            kernel32.CloseHandle(theirs_handle)
        ours = _integrity_level(self._advapi32, kernel32.GetCurrentProcess())
        if theirs < 0 or ours < 0:
            return None
        return theirs > ours

    def _on_mouse(self, code, wparam, lparam):
        try:
            if code == HC_ACTION and wparam in (WM_MOUSEWHEEL, WM_MOUSEHWHEEL):
                data = ctypes.cast(lparam, ctypes.POINTER(MSLLHOOKSTRUCT)).contents
                if self._handle(int(wparam), data):
                    return 1
        except Exception:
            # A hook that raises is a hook Windows may drop, taking the mouse
            # with it. Any failure here means the notch goes where it would
            # have gone without this class.
            pass
        return self._user32.CallNextHookEx(None, code, wparam, lparam)

    def _handle(self, message: int, data) -> bool:
        """True when the notch has been dealt with and must not go on."""
        region = self._region
        if region.covers_point(data.pt.x, data.pt.y):
            self._forward(message, data)
            return True
        if self._tracked is None:
            return False
        x, y, w, h = self._tracked
        return not (x <= data.pt.x < x + w and y <= data.pt.y < y + h)

    def _forward(self, message: int, data) -> None:
        """Hand the notch to the surface, in the shape its window proc expects.

        The delta is the high word of `mouseData` and the modifier keys are the
        low word, packed back the way `WM_MOUSEWHEEL` carries them; the position
        is in screen coordinates, which is what the message wants and what the
        hook already has.
        """
        hwnd = self._region.hwnd
        if not hwnd:
            return
        wparam = data.mouseData & 0xFFFFFFFF
        lparam = ((data.pt.y & 0xFFFF) << 16) | (data.pt.x & 0xFFFF)
        self._user32.PostMessageW(hwnd, message, wparam, lparam)


def _integrity_api():
    """The calls that answer what integrity level a process runs at.

    Separate from the ones the hook itself needs because a machine where these
    cannot be bound is one where the hook still works — the question simply goes
    unanswered, and the page hears nothing rather than a wrong claim.
    """
    try:
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        advapi32 = ctypes.WinDLL("advapi32", use_last_error=True)
    except OSError:
        return None
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.GetCurrentProcess.restype = wintypes.HANDLE
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    advapi32.OpenProcessToken.argtypes = [
        wintypes.HANDLE, wintypes.DWORD, ctypes.POINTER(wintypes.HANDLE),
    ]
    advapi32.GetTokenInformation.argtypes = [
        wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD,
        ctypes.POINTER(wintypes.DWORD),
    ]
    advapi32.GetSidSubAuthorityCount.restype = ctypes.POINTER(ctypes.c_ubyte)
    advapi32.GetSidSubAuthorityCount.argtypes = [ctypes.c_void_p]
    advapi32.GetSidSubAuthority.restype = ctypes.POINTER(wintypes.DWORD)
    advapi32.GetSidSubAuthority.argtypes = [ctypes.c_void_p, wintypes.DWORD]
    return kernel32, advapi32


def _integrity_level(api, process) -> int:
    """The mandatory integrity level of an open process handle, or -1.

    The level is the last sub-authority of the token's integrity SID: 0x2000 is
    what an ordinary process gets, 0x3000 what one started as administrator
    does. Only the comparison between two of them is used, so the constants stay
    out of this module.
    """
    kernel32, advapi32 = api
    token = wintypes.HANDLE()
    if not advapi32.OpenProcessToken(process, TOKEN_QUERY, ctypes.byref(token)):
        return -1
    try:
        size = wintypes.DWORD()
        advapi32.GetTokenInformation(
            token, TOKEN_INTEGRITY_LEVEL, None, 0, ctypes.byref(size)
        )
        if ctypes.get_last_error() != ERROR_INSUFFICIENT_BUFFER or not size.value:
            return -1
        buf = ctypes.create_string_buffer(size.value)
        if not advapi32.GetTokenInformation(
            token, TOKEN_INTEGRITY_LEVEL, buf, size.value, ctypes.byref(size)
        ):
            return -1
        sid = ctypes.cast(buf, ctypes.POINTER(TOKEN_MANDATORY_LABEL)).contents.Label.Sid
        count = advapi32.GetSidSubAuthorityCount(sid)
        if not count:
            return -1
        return int(advapi32.GetSidSubAuthority(sid, count[0] - 1)[0])
    finally:
        kernel32.CloseHandle(token)
