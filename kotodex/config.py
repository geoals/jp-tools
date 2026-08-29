"""The names more than one of these modules needs.

Its own module because the entry script cannot be imported: under PyInstaller
`kotodex.py` is bundled as `__main__` alone, so `from kotodex import ...` fails
there — and on Linux it silently runs a second copy of it.
"""

import json
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


# Read off `SERVER_URL` rather than assumed, so overriding the URL moves both
# halves of the probe below together.
_split = urllib.parse.urlsplit(SERVER_URL)
_ADDRESS = (_split.hostname or "127.0.0.1", _split.port or SERVER_PORT)


def kotodex_server_up() -> bool:
    """Whether the server is answering — without paying a timeout to learn it is not.

    The connect is a stage of its own because **a closed port does not always
    refuse**. Where it drops instead, one generous timeout is paid in full on
    every negative check, and the launcher makes that check before it starts
    anything. So the connect gets a deadline short enough that learning there is
    no listener is cheap, and only the answer keeps a generous one — a server that
    is up but busy still gets its two seconds to reply.
    """
    try:
        with socket.create_connection(_ADDRESS, timeout=0.3):
            pass
    except OSError:
        return False
    try:
        with _OPENER.open(f"{SERVER_URL}/api/reader/state", timeout=2) as r:
            return r.status == 200
    except (urllib.error.URLError, OSError):
        return False


def setup_blocked() -> bool | None:
    """Whether the server says a part reading needs is missing.

    `None` when it cannot be asked — not yet up, or the probe failed. The caller
    is polling, so "not known yet" and "nothing missing" have to be different
    answers.

    The probe is the server's, never the launcher's: `/api/setup` is the one
    place that decides what blocks, and a second opinion here would be a rule to
    keep in step with it.
    """
    try:
        with _OPENER.open(f"{SERVER_URL}/api/setup", timeout=5) as r:
            caps = json.load(r)
    except (urllib.error.URLError, OSError, ValueError):
        return None
    return any(
        isinstance(c, dict) and c.get("blocking") and not c.get("ok")
        for c in caps.values()
    )
