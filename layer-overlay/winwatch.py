"""Where the tracked window is, told by Windows rather than asked again and again.

Windows-only: `ctypes.wintypes` does not import anywhere else. [`layer_overlay`]
imports this or [`xwatch`] by backend, and the two answer the same questions.

The same bargain as [`xwatch`], for the same reason: polling a window's position
is wrong for up to an interval after every move, and the interval is a choice
between being wrong for longer and spending more to be wrong for less. Windows
will say when a window moves, through `SetWinEventHook`.

Two differences from the X11 watcher, both consequences of how the events arrive:

- There is no file descriptor. A `WINEVENT_OUTOFCONTEXT` hook is delivered
  through the thread's own message queue, which Qt is already pumping, so the
  hook procedure is simply called and there is nothing for an event loop to wait
  on. [`fd`] is None, and [`on_change`] is called instead.
- The hook is scoped to the tracked window's thread, not global. A global
  `EVENT_OBJECT_LOCATIONCHANGE` hook hears every window on the desktop and the
  cursor besides; one thread's events are a handful. So there is no hook until a
  window has been found, which leaves *discovery* to the caller's poll — the same
  division as under X11, where rediscovery is rate-limited and only the following
  is event-driven.

Nothing here reads an event's contents beyond checking that it is about a window
rather than a caret or the cursor. Any such event means "ask again", and asking is
one call that does not leave the machine.

**An upscaler between the game and the screen is what the reader is looking at,
so that is what gets followed.** Magpie draws a scaled copy of the game into a
window of its own and leaves the game's own window where it was, unscaled and
usually behind. Following the game there puts the strip over the small original
rather than over the picture, and every `--text-*` fraction is measured against
the wrong rectangle. Its window carries **no title**, so it cannot be found the
way a game is; the class name is the only handle on it, and the GUID in it is
Magpie's own fixed constant. The *client* area is taken rather than the frame,
because the frame includes a border that is not part of the picture.
"""

import ctypes
from ctypes import wintypes

MAGPIE_SCALING_CLASS = "Window_Magpie_967EB565-6F73-4E94-AE53-00CC42592A22"

EVENT_OBJECT_DESTROY = 0x8001
EVENT_OBJECT_LOCATIONCHANGE = 0x800B
WINEVENT_OUTOFCONTEXT = 0x0000

#: The event's `idObject` for the window itself. A caret, a cursor and a
#: scrollbar all report location changes too, under their own negative ids.
OBJID_WINDOW = 0

_WINEVENTPROC = ctypes.WINFUNCTYPE(
    None,
    wintypes.HANDLE,   # hook
    wintypes.DWORD,    # event
    wintypes.HWND,
    wintypes.LONG,     # idObject
    wintypes.LONG,     # idChild
    wintypes.DWORD,    # thread
    wintypes.DWORD,    # time
)

_ENUMPROC = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)


class WindowWatch:
    """Follows the first top-level window whose title contains a substring.

    Top-level only, unlike the X11 walk: there is no reparenting into a window
    manager's frame here, so a game's title is on the window whose rectangle is
    wanted.
    """

    def __init__(self) -> None:
        self._name = ""
        self._window = 0
        self._scaling = 0
        self._rect = None
        self._hook = 0
        #: Called when an event says the window moved. Set by the caller; the
        #: default keeps this usable on its own.
        self.on_change = lambda: None
        user32 = ctypes.WinDLL("user32", use_last_error=True)
        user32.EnumWindows.argtypes = [_ENUMPROC, wintypes.LPARAM]
        user32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
        user32.GetWindowTextLengthW.argtypes = [wintypes.HWND]
        user32.IsWindowVisible.argtypes = [wintypes.HWND]
        user32.GetWindowRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT)]
        user32.GetClientRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT)]
        user32.ClientToScreen.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.POINT)]
        user32.FindWindowExW.restype = wintypes.HWND
        user32.FindWindowExW.argtypes = [
            wintypes.HWND, wintypes.HWND, wintypes.LPCWSTR, wintypes.LPCWSTR,
        ]
        user32.GetWindowThreadProcessId.argtypes = [
            wintypes.HWND, ctypes.POINTER(wintypes.DWORD)
        ]
        user32.SetWinEventHook.restype = wintypes.HANDLE
        user32.SetWinEventHook.argtypes = [
            wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE, _WINEVENTPROC,
            wintypes.DWORD, wintypes.DWORD, wintypes.DWORD,
        ]
        user32.UnhookWinEvent.argtypes = [wintypes.HANDLE]
        self._user32 = user32
        # Held as an attribute because the system keeps only the pointer: a
        # callback Python has collected is a crash the next time a window moves.
        self._proc = _WINEVENTPROC(self._on_event)

    @property
    def available(self) -> bool:
        """True: user32 is part of the platform. The X11 watcher can genuinely
        fail to open a display, and the caller asks both the same question."""
        return True

    @property
    def fd(self) -> None:
        """No descriptor — see the module docstring. The caller reads this to
        tell which of the two delivery shapes it has."""
        return None

    def set_name(self, name: str) -> None:
        """Track the first window whose title contains `name`. Empty: none."""
        self._name = name or ""
        self._release()
        self._rect = None
        if self._name:
            self.refresh()

    def _release(self) -> None:
        """Drop the window and the hook that was scoped to its thread.

        Deliberately leaves `_rect` alone: losing the window is a change the
        caller has to hear about, and [`refresh`] reports it by comparing the new
        answer against the old one.
        """
        self._window = 0
        if self._hook:
            self._user32.UnhookWinEvent(self._hook)
            self._hook = 0

    def _on_event(self, hook, event, hwnd, id_object, id_child, thread, time) -> None:
        """A window on the tracked thread changed. Which one matters; what it did
        does not.

        The hook is scoped to a thread rather than a window, so a game's other
        windows — a splash, a tooltip — arrive here too and must not be read as
        the tracked one moving.
        """
        if id_object != OBJID_WINDOW or hwnd != self._window:
            return
        self.on_change()

    def _find(self) -> int:
        """The first visible top-level window whose title contains the name."""
        found = 0

        def visit(hwnd, _):
            nonlocal found
            if not self._user32.IsWindowVisible(hwnd):
                return True
            length = self._user32.GetWindowTextLengthW(hwnd)
            if not length:
                return True
            title = ctypes.create_unicode_buffer(length + 1)
            self._user32.GetWindowTextW(hwnd, title, length + 1)
            if self._name in title.value:
                found = hwnd
                return False
            return True

        self._user32.EnumWindows(_ENUMPROC(visit), 0)
        return found

    def _geometry(self, window: int):
        """`x, y, width, height` in screen coordinates, or None if it is gone.

        Already absolute, unlike X11's, where the answer is relative to a parent
        that is usually the window manager's frame.
        """
        frame = wintypes.RECT()
        if not self._user32.GetWindowRect(window, ctypes.byref(frame)):
            return None
        w = frame.right - frame.left
        h = frame.bottom - frame.top
        if w <= 0 or h <= 0:
            return None
        return (frame.left, frame.top, w, h)

    def _upscaler(self) -> int:
        """The upscaler's output window, 0 when nothing is upscaling.

        By class, because it has no title — see the module docstring. One lookup
        against the window class table rather than an `EnumWindows` walk, so this
        can run on every refresh and notice one appearing mid-session.

        Visibility is checked for the same reason [`_find`] checks it: a window
        that exists and is not on screen is not what the reader is looking at, and
        following its rectangle would move the strip somewhere nothing is drawn.
        """
        window = self._user32.FindWindowExW(None, None, MAGPIE_SCALING_CLASS, None)
        if not window or not self._user32.IsWindowVisible(window):
            return 0
        return window

    def _client_geometry(self, window: int):
        """The window's client area as `x, y, width, height` in screen coordinates.

        `GetClientRect` answers in client coordinates, where the origin is always
        zero, so it gives the size and says nothing about where — the origin has
        to be mapped separately.
        """
        client = wintypes.RECT()
        if not self._user32.GetClientRect(window, ctypes.byref(client)):
            return None
        origin = wintypes.POINT(client.left, client.top)
        if not self._user32.ClientToScreen(window, ctypes.byref(origin)):
            return None
        w = client.right - client.left
        h = client.bottom - client.top
        if w <= 0 or h <= 0:
            return None
        return (origin.x, origin.y, w, h)

    def refresh(self) -> bool:
        """Ask where the window is now. True if that is different."""
        if not self._name:
            was, self._rect = self._rect, None
            return was is not None
        if not self._window:
            self._window = self._find()
            if self._window:
                thread = self._user32.GetWindowThreadProcessId(self._window, None)
                if thread:
                    self._hook = self._user32.SetWinEventHook(
                        EVENT_OBJECT_DESTROY, EVENT_OBJECT_LOCATIONCHANGE,
                        None, self._proc, 0, thread, WINEVENT_OUTOFCONTEXT,
                    )
        rect = None
        self._scaling = 0
        if self._window:
            scaling = self._upscaler()
            if scaling:
                rect = self._client_geometry(scaling)
                if rect is not None:
                    self._scaling = scaling
            if rect is None:
                rect = self._geometry(self._window)
        if rect is None:
            # Gone, or never found. Either way the next call rediscovers.
            self._release()
        if rect == self._rect:
            return False
        self._rect = rect
        return True

    def aim(self, x: int, y: int):
        """Where the reader is pointing, or None when that is where the cursor is.

        An upscaler fences the cursor inside the window it is scaling — that is
        how the game goes on receiving mouse input from under a picture drawn
        somewhere else — and draws its own cursor at the matching place in the
        picture. So the reader points at the picture while the cursor Windows
        reports, and the click it will deliver, are back in the small original.
        The two agree at one point and diverge by the scale everywhere else.

        Same scale as the picture, because it is the same mapping: source client
        to upscaler client, both in screen coordinates.
        """
        if not self._scaling or not self._window:
            return None
        source = self._client_geometry(self._window)
        picture = self._client_geometry(self._scaling)
        if source is None or picture is None:
            return None
        sx, sy, sw, sh = source
        px, py, pw, ph = picture
        if not (sx <= x < sx + sw and sy <= y < sy + sh) or sw <= 0 or sh <= 0:
            return None
        return (
            round(px + (x - sx) * pw / sw),
            round(py + (y - sy) * ph / sh),
        )

    @property
    def rect(self):
        """The window as `x, y, width, height`, or None while there is none."""
        return self._rect

    @property
    def window(self) -> int:
        """The tracked window's handle, 0 while there is none.

        Read by [`winfocus`], which asks whose process is in front rather than
        where anything is.
        """
        return self._window
