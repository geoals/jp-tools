"""Where the tracked window is, told by X rather than asked again and again.

Polling a window's position is the wrong shape for the question. It costs a
subprocess and two round trips whichever answer it gets, it is wrong for up to
an interval after every move, and the interval is a choice between being wrong
for longer and spending more to be wrong for less. X already knows when a
window moves and will say so.

So: `SubstructureNotifyMask` on the root, which reports every toplevel being
configured, mapped, unmapped or destroyed, and `StructureNotifyMask` on the
window itself. Both together because a managed window is reparented into a
frame of the window manager's own — dragging it moves the *frame*, and the
client window below it never moves relative to its parent, so only the root's
substructure hears about it.

Nothing here parses an event. Any event at all means "ask again", and asking is
two round trips on a connection already open. What the events buy is knowing
*when*, which is the whole difference between following a window and sampling
where it used to be.

The connection is this module's own — see [`xshape`] for why a second one is
not worth avoiding — and it exposes its file descriptor so a Qt event loop can
wait on it instead of waking on a timer.
"""

import ctypes
import ctypes.util
import time

SUBSTRUCTURE_NOTIFY = 1 << 19
STRUCTURE_NOTIFY = 1 << 17

#: An `XEvent` is a union whose largest member is a fixed run of longs. Nothing
#: here reads a field, so the size is all that matters.
_EVENT_WORDS = 32

#: Rediscovery walks the whole window tree, and a burst of root events would
#: otherwise walk it once per event.
DISCOVER_INTERVAL = 0.25


class WindowWatch:
    """Follows the first window whose title contains a substring.

    Not a general X binding: the two questions it answers are "which window is
    it" and "where is it now", and it answers the second from events.
    """

    def __init__(self) -> None:
        self._x11 = None
        self._display = None
        self._root = 0
        self._name = ""
        self._window = 0
        self._rect = None
        self._discovered_at = 0.0
        x11_name = ctypes.util.find_library("X11")
        if not x11_name:
            return
        try:
            x11 = ctypes.CDLL(x11_name)
        except OSError:
            return
        self._bind(x11)
        display = x11.XOpenDisplay(None)
        if not display:
            return
        self._x11 = x11
        self._display = display
        self._root = x11.XDefaultRootWindow(ctypes.c_void_p(display))
        # A window can be destroyed between being found and being asked about,
        # and Xlib's default error handler exits the process for it. Every call
        # here is allowed to fail and be treated as "gone".
        self._errors = self._ERROR_HANDLER(lambda *_: 0)
        x11.XSetErrorHandler(self._errors)
        x11.XSelectInput(
            ctypes.c_void_p(display), ctypes.c_ulong(self._root),
            ctypes.c_long(SUBSTRUCTURE_NOTIFY),
        )
        self._utf8_name = self._atom("_NET_WM_NAME")
        self._utf8_string = self._atom("UTF8_STRING")
        self._wm_name = self._atom("WM_NAME")

    _ERROR_HANDLER = ctypes.CFUNCTYPE(
        ctypes.c_int, ctypes.c_void_p, ctypes.c_void_p
    )

    def _bind(self, x11) -> None:
        x11.XOpenDisplay.restype = ctypes.c_void_p
        x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
        x11.XDefaultRootWindow.restype = ctypes.c_ulong
        x11.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
        x11.XConnectionNumber.restype = ctypes.c_int
        x11.XConnectionNumber.argtypes = [ctypes.c_void_p]
        x11.XPending.restype = ctypes.c_int
        x11.XPending.argtypes = [ctypes.c_void_p]
        x11.XNextEvent.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
        x11.XSelectInput.argtypes = [
            ctypes.c_void_p, ctypes.c_ulong, ctypes.c_long,
        ]
        x11.XInternAtom.restype = ctypes.c_ulong
        x11.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
        x11.XFree.argtypes = [ctypes.c_void_p]
        x11.XFlush.argtypes = [ctypes.c_void_p]
        x11.XSetErrorHandler.restype = ctypes.c_void_p
        x11.XQueryTree.argtypes = [
            ctypes.c_void_p, ctypes.c_ulong,
            ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)),
            ctypes.POINTER(ctypes.c_uint),
        ]
        x11.XGetGeometry.argtypes = [
            ctypes.c_void_p, ctypes.c_ulong,
            ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint),
            ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint),
        ]
        x11.XTranslateCoordinates.argtypes = [
            ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong,
            ctypes.c_int, ctypes.c_int,
            ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_ulong),
        ]
        x11.XGetWindowProperty.argtypes = [
            ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong,
            ctypes.c_long, ctypes.c_long, ctypes.c_int, ctypes.c_ulong,
            ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.POINTER(ctypes.c_ubyte)),
        ]

    @property
    def available(self) -> bool:
        return bool(self._display)

    @property
    def fd(self) -> int:
        """The connection, for an event loop to wait on."""
        return self._x11.XConnectionNumber(ctypes.c_void_p(self._display))

    def _atom(self, name: str) -> int:
        return self._x11.XInternAtom(
            ctypes.c_void_p(self._display), name.encode(), 0
        )

    def set_name(self, name: str) -> None:
        """Track the first window whose title contains `name`. Empty: none."""
        self._name = name or ""
        self._window = 0
        self._rect = None
        self._discovered_at = 0.0
        if self._name:
            self.refresh()

    def _title(self, window: int) -> str:
        """`_NET_WM_NAME` if the window has one, else `WM_NAME`.

        Both, because the second is what a program that predates the first
        sets, and a Wine game can be either.
        """
        for prop, kind in (
            (self._utf8_name, self._utf8_string),
            (self._wm_name, 0),  # AnyPropertyType
        ):
            actual_type = ctypes.c_ulong()
            actual_format = ctypes.c_int()
            nitems = ctypes.c_ulong()
            bytes_after = ctypes.c_ulong()
            data = ctypes.POINTER(ctypes.c_ubyte)()
            ok = self._x11.XGetWindowProperty(
                ctypes.c_void_p(self._display), ctypes.c_ulong(window),
                ctypes.c_ulong(prop), 0, 1024, 0, ctypes.c_ulong(kind),
                ctypes.byref(actual_type), ctypes.byref(actual_format),
                ctypes.byref(nitems), ctypes.byref(bytes_after),
                ctypes.byref(data),
            )
            if ok != 0 or not data:
                continue
            raw = bytes(bytearray(data[: nitems.value]))
            self._x11.XFree(data)
            if raw:
                return raw.decode("utf-8", "replace")
        return ""

    def _find(self, window: int, depth: int = 0) -> int:
        """The first window at or under `window` whose title matches.

        Depth-limited: a match lives on a toplevel or on the client window
        inside the window manager's frame, and walking deeper than that is
        walking into a program's own widgets.
        """
        if depth and self._name in self._title(window):
            return window
        if depth > 3:
            return 0
        root = ctypes.c_ulong()
        parent = ctypes.c_ulong()
        children = ctypes.POINTER(ctypes.c_ulong)()
        count = ctypes.c_uint()
        ok = self._x11.XQueryTree(
            ctypes.c_void_p(self._display), ctypes.c_ulong(window),
            ctypes.byref(root), ctypes.byref(parent),
            ctypes.byref(children), ctypes.byref(count),
        )
        if not ok or not children:
            return 0
        found = 0
        for i in range(count.value):
            found = self._find(children[i], depth + 1)
            if found:
                break
        self._x11.XFree(children)
        return found

    def _geometry(self, window: int):
        """`x, y, width, height` in root coordinates, or None if it is gone.

        Translated rather than taken from `XGetGeometry`, whose x and y are
        relative to the parent — which for a managed window is the window
        manager's frame, not the screen.
        """
        root = ctypes.c_ulong()
        x = ctypes.c_int()
        y = ctypes.c_int()
        w = ctypes.c_uint()
        h = ctypes.c_uint()
        border = ctypes.c_uint()
        depth = ctypes.c_uint()
        ok = self._x11.XGetGeometry(
            ctypes.c_void_p(self._display), ctypes.c_ulong(window),
            ctypes.byref(root), ctypes.byref(x), ctypes.byref(y),
            ctypes.byref(w), ctypes.byref(h),
            ctypes.byref(border), ctypes.byref(depth),
        )
        if not ok or not w.value or not h.value:
            return None
        abs_x = ctypes.c_int()
        abs_y = ctypes.c_int()
        child = ctypes.c_ulong()
        ok = self._x11.XTranslateCoordinates(
            ctypes.c_void_p(self._display), ctypes.c_ulong(window),
            ctypes.c_ulong(self._root), 0, 0,
            ctypes.byref(abs_x), ctypes.byref(abs_y), ctypes.byref(child),
        )
        if not ok:
            return None
        return (abs_x.value, abs_y.value, w.value, h.value)

    def drain(self) -> None:
        """Take everything the connection is holding, and read nothing from it.

        The events are the clock, not the data — see the module docstring.
        """
        if not self.available:
            return
        event = (ctypes.c_long * _EVENT_WORDS)()
        while self._x11.XPending(ctypes.c_void_p(self._display)) > 0:
            self._x11.XNextEvent(ctypes.c_void_p(self._display), ctypes.byref(event))

    def pending(self) -> int:
        """Events already taken off the socket and waiting in Xlib's queue.

        Asking X anything reads from the connection, so replies arrive mixed
        with events and the events land here. A reader waiting only on the file
        descriptor would not be woken for these — the bytes are already read.
        """
        if not self.available:
            return 0
        return self._x11.XPending(ctypes.c_void_p(self._display))

    def refresh(self) -> bool:
        """Ask where the window is now. True if that is different.

        Rediscovery is rate-limited and only happens with nothing tracked: with
        a window in hand this is two round trips, which is cheap enough to do
        on every event that a drag produces.
        """
        if not self.available or not self._name:
            was, self._rect = self._rect, None
            return was is not None
        if not self._window:
            now = time.monotonic()
            if now - self._discovered_at < DISCOVER_INTERVAL:
                return False
            self._discovered_at = now
            self._window = self._find(self._root)
            if self._window:
                # Its own configure events as well as the frame's: a window
                # that resizes without its frame moving is still a resize.
                self._x11.XSelectInput(
                    ctypes.c_void_p(self._display), ctypes.c_ulong(self._window),
                    ctypes.c_long(STRUCTURE_NOTIFY),
                )
                self._x11.XFlush(ctypes.c_void_p(self._display))
        rect = self._geometry(self._window) if self._window else None
        if rect is None:
            # Gone, or never found. Either way the next call rediscovers.
            self._window = 0
        if rect == self._rect:
            return False
        self._rect = rect
        return True

    @property
    def rect(self):
        """The window as `x, y, width, height`, or None while there is none."""
        return self._rect
