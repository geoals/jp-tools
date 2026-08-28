"""The names more than one of these modules needs.

Its own module because the entry script cannot be imported: under PyInstaller
`kotodex.py` is bundled as `__main__` alone, so `from kotodex import ...` fails
there — and on Linux it silently runs a second copy of it.
"""

import os
import socket
import urllib.error
import urllib.parse
import urllib.request

SERVER_PORT = int(os.environ.get("KOTODEX_SERVER_PORT", "3200"))
# Numeric: `localhost` resolves to `::1` first on Windows, where the server binds
# IPv4 only, and a connection to `::1` there times out rather than being refused.
SERVER_URL = os.environ.get("KOTODEX_SERVER_URL", f"http://127.0.0.1:{SERVER_PORT}")

# Reverse-DNS off kotodex.com, and the same string as the desktop entry's
# filename: on Wayland Qt uses it as the app_id, which is how the compositor
# matches the window to the entry.
APP_ID = "com.kotodex.Kotodex"
SOCKET_NAME = APP_ID


# No proxy: the server is on this machine. Windows takes its proxy from the system
# settings, and resolving one cost the first request 1.6 seconds — longer than the
# probe's own timeout, so every poll timed out and the launcher waited out its
# whole deadline against a server that was answering. Through the opener it is
# 0.01s. The same trap the overlay's own checks are built around.
_OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))


# Where `SERVER_URL` points, for the connect below. Taken apart rather than
# assumed, so overriding the URL moves both halves of the probe together.
_SPLIT = urllib.parse.urlsplit(SERVER_URL)
_ADDRESS = (_SPLIT.hostname or "127.0.0.1", _SPLIT.port or SERVER_PORT)

#: Long enough for a listener on this machine, which accepts in about a
#: millisecond, and short enough that learning there is none is cheap.
CONNECT_TIMEOUT = 0.3


def kotodex_server_up() -> bool:
    """Whether the server is answering — without paying a timeout to learn it is not.

    The connect is separate because **a closed port does not always refuse**. It
    can drop, and then one generous timeout is paid in full on every negative
    check: the launcher's probe of a port nothing was listening on cost two
    seconds of every Windows start, before the first component was spawned. So
    the connect gets a short deadline and only the answer keeps a generous one —
    a server that is up but busy still gets its two seconds to reply.
    """
    try:
        with socket.create_connection(_ADDRESS, timeout=CONNECT_TIMEOUT):
            pass
    except OSError:
        return False
    try:
        with _OPENER.open(f"{SERVER_URL}/api/reader/state", timeout=2) as r:
            return r.status == 200
    except (urllib.error.URLError, OSError):
        return False
