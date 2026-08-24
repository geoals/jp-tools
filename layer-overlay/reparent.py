"""Making the surface a child of the window it is drawn over.

An X child window is stacked with its parent, clipped to it, and moved by the
server when the parent moves. All three are what an overlay over a game wants,
and all three are free — there is nothing to poll and nothing to follow. It is
also what puts the surface above a *fullscreen* game without layer-shell: the
child inherits whatever stacking layer the parent is in.

Both the game and the surface have to be X clients for this. Under a Wayland
session that means XWayland on both sides, which Wine and Proton give for free
and which `backend.apply_environment` arranges for this end.

Done through ctypes for the same reason as [`xshape`]: libX11 is present
wherever X is, and this needs six calls from it. Its own connection, again —
Qt does not hand out the `Display *`.
"""

import ctypes
import ctypes.util


class Reparenter:
    """Moves a window between the root and a foreign parent, on `DISPLAY`.

    Every call flushes: nothing else on this connection will.
    """

    def __init__(self) -> None:
        self._x11 = None
        self._display = None
        self._root = 0
        name = ctypes.util.find_library("X11")
        if not name:
            return
        try:
            x11 = ctypes.CDLL(name)
        except OSError:
            return
        x11.XOpenDisplay.restype = ctypes.c_void_p
        x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
        x11.XDefaultRootWindow.restype = ctypes.c_ulong
        x11.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
        x11.XReparentWindow.argtypes = [
            ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong,
            ctypes.c_int, ctypes.c_int,
        ]
        x11.XMoveResizeWindow.argtypes = [
            ctypes.c_void_p, ctypes.c_ulong,
            ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint,
        ]
        x11.XMapRaised.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        x11.XRaiseWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        x11.XFlush.argtypes = [ctypes.c_void_p]
        display = x11.XOpenDisplay(None)
        if not display:
            return
        self._x11 = x11
        self._display = display
        self._root = x11.XDefaultRootWindow(ctypes.c_void_p(display))

    @property
    def available(self) -> bool:
        return bool(self._display)

    def _flush(self) -> None:
        self._x11.XFlush(ctypes.c_void_p(self._display))

    def into(self, child: int, parent: int, width: int, height: int) -> bool:
        """Put `child` inside `parent`, filling it.

        X unmaps a mapped window before reparenting it, so the map afterwards is
        required rather than belt-and-braces — without it the surface simply
        disappears into a parent it is correctly a child of. Raised as well: the
        game may have children of its own, and the overlay belongs on top of
        them.
        """
        if not self.available or not child or not parent:
            return False
        self._x11.XReparentWindow(
            ctypes.c_void_p(self._display),
            ctypes.c_ulong(child), ctypes.c_ulong(parent), 0, 0,
        )
        self.fill(child, width, height)
        self._x11.XMapRaised(ctypes.c_void_p(self._display), ctypes.c_ulong(child))
        self._flush()
        return True

    def fill(self, child: int, width: int, height: int) -> None:
        """Match the parent's size. Coordinates are parent-relative once inside."""
        if not self.available or not child:
            return
        self._x11.XMoveResizeWindow(
            ctypes.c_void_p(self._display), ctypes.c_ulong(child),
            0, 0, max(int(width), 1), max(int(height), 1),
        )
        self._flush()

    def to_root(self, child: int, x: int, y: int, width: int, height: int) -> None:
        """Back to a toplevel, where the game has gone away.

        Mapped again for the same reason as [`into`]. Nothing here asks the
        window manager to manage it: the surface is override-redirect in this
        mode, so it goes back to being an unmanaged window on top.
        """
        if not self.available or not child:
            return
        self._x11.XReparentWindow(
            ctypes.c_void_p(self._display),
            ctypes.c_ulong(child), ctypes.c_ulong(self._root), int(x), int(y),
        )
        self._x11.XMoveResizeWindow(
            ctypes.c_void_p(self._display), ctypes.c_ulong(child),
            int(x), int(y), max(int(width), 1), max(int(height), 1),
        )
        self._x11.XMapRaised(ctypes.c_void_p(self._display), ctypes.c_ulong(child))
        self._flush()
