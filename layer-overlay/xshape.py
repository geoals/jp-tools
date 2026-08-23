"""The X11 input region, which is not what `QWindow.setMask` sets.

Under Wayland Qt maps a window mask onto `wl_surface.set_input_region` and the
surface keeps drawing everywhere. Under X11 it maps it onto the **bounding**
shape, which clips what the window draws — so the same call that passes clicks
through on layer-shell would leave an X11 overlay visible only where it is
clickable.

`ShapeInput` is the one that means "clicks land here"; the bounding shape stays
whole. Done through ctypes rather than python-xlib: libXext is present wherever
X is, and this needs four calls from it.
"""

import ctypes
import ctypes.util

SHAPE_INPUT = 2
SHAPE_SET = 0
UNSORTED = 0


class _XRectangle(ctypes.Structure):
    _fields_ = [
        ("x", ctypes.c_short),
        ("y", ctypes.c_short),
        ("width", ctypes.c_ushort),
        ("height", ctypes.c_ushort),
    ]


class InputRegion:
    """Sets the input shape of a window on the X display named by `DISPLAY`.

    Its own connection, not Qt's: Qt does not expose the `Display *`, and a
    second connection is cheap. Every change has to be flushed — nothing else on
    this connection will do it.
    """

    def __init__(self) -> None:
        self._x11 = None
        self._xext = None
        self._display = None
        x11_name = ctypes.util.find_library("X11")
        xext_name = ctypes.util.find_library("Xext")
        if not x11_name or not xext_name:
            return
        try:
            self._x11 = ctypes.CDLL(x11_name)
            self._xext = ctypes.CDLL(xext_name)
        except OSError:
            self._x11 = self._xext = None
            return
        self._x11.XOpenDisplay.restype = ctypes.c_void_p
        self._display = self._x11.XOpenDisplay(None)

    @property
    def available(self) -> bool:
        return bool(self._display)

    def apply(self, window_id: int, rects) -> bool:
        """`rects` is a sequence of `(x, y, w, h)`. Empty means nothing clickable.

        An empty rectangle list is the honest encoding of "no clickable area"
        here, unlike the Wayland side where an empty *mask* means the opposite.
        """
        if not self.available or not window_id:
            return False
        array = (_XRectangle * len(rects))()
        for i, (x, y, w, h) in enumerate(rects):
            array[i] = _XRectangle(int(x), int(y), max(int(w), 1), max(int(h), 1))
        self._xext.XShapeCombineRectangles(
            ctypes.c_void_p(self._display),
            ctypes.c_ulong(window_id),
            ctypes.c_int(SHAPE_INPUT),
            ctypes.c_int(0),
            ctypes.c_int(0),
            array,
            ctypes.c_int(len(rects)),
            ctypes.c_int(SHAPE_SET),
            ctypes.c_int(UNSORTED),
        )
        self._x11.XFlush(ctypes.c_void_p(self._display))
        return True
