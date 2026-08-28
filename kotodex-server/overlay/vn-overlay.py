#!/usr/bin/env python3
"""The line being read, as a strip over the VN, fullscreen included.

The page is `overlay.html` beside this file — the newest line, and a dictionary
popup for the word clicked in it. Yomitan does not run here (QtWebEngine loads
no extensions), so the popup is the page's own.

kotodex-server is the backend, and serves the page from this directory: it calls
`/api` for the line stream, the dictionary, the ledger and the card, and none of
that can be answered locally. Starting without kotodex-server gets a warning, not a
failure — `EventSource` reconnects on its own, so the strip fills in when
kotodex-server arrives.

The surface, the input region and the window tracking are `layer-overlay`, which
knows nothing about any of this. What stays here is the URL, the two health
checks, and `--mobile`.

`SIGUSR1` makes the whole surface take input, for selecting text rather than
advancing the VN. `SIGUSR2` toggles ghost mode: the line is laid over the game's
own text and drawn invisibly, so what is read is the game's typesetting and all
this adds is the status tint per word and somewhere to click. Bind either to a
KDE shortcut:

    pkill -USR1 -f vn-overlay.py
    pkill -USR2 -f vn-overlay.py

`--mobile` is the overlay read off a phone: everything at 1.75x, the line on
the bottom edge, and the popup carrying known / unknown / mine buttons, since
driving the PC's mouse from the phone leaves no side buttons for them. The
strip grows with the type; `VN_OVERLAY_HEIGHT` still wins if it is set.

    VN_OVERLAY_URL      page to show      (default overlay.html, over kotodex-server)
    KOTODEX_ANKI_URL   AnkiConnect       (default http://127.0.0.1:8765)
    VN_OVERLAY_HEIGHT   strip height, px  (default 300, 525 with --mobile)
    VN_OVERLAY_BG       backdrop alpha    (default 0.82)
    VN_OVERLAY_FONT     font for the line (vn-overlay.sh sets DNP Shuei Mincho
                        Pr6; unset falls back to the page's Noto Sans CJK JP)
"""

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import NamedTuple
from urllib.parse import quote, urlsplit

# layer-overlay is a sibling directory rather than an installed package, so it has
# to be put on the path. Not when frozen: there is no repository beside the
# executable then, and the module is already inside the bundle.
if not getattr(sys, "frozen", False):
    sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "layer-overlay"))
import layer_overlay  # noqa: E402

DEFAULT_URL = "http://localhost:3200/overlay/overlay.html"
# The launcher's single-instance socket, which its ✕ writes to. Named here as
# well as in `kotodex/config.py`: one string, and the launcher owns it.
LAUNCHER_SOCKET = "com.kotodex.Kotodex"
ANKI_URL = os.environ.get("KOTODEX_ANKI_URL", "http://127.0.0.1:8765")
# Names the surface to the compositor and the page's localStorage.
SCOPE = "vn-overlay"
# Beside the databases, which is what jp_core::install::data_dir answers:
# LOCALAPPDATA on Windows, XDG_DATA_HOME or its default elsewhere.
_DATA_ROOT = (
    Path(os.environ["LOCALAPPDATA"])
    if sys.platform == "win32"
    else Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share")
)
STORAGE = _DATA_ROOT / "kotodex/overlay"

# No proxy for either check below: both addresses are on this machine. Windows
# takes its proxy from the system settings, and one configured for the internet
# swallowed the localhost request and reported kotodex-server as down.
_OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))


class _Check(NamedTuple):
    seconds: float
    error: str


def _timed_check(target) -> _Check:
    """Open `target`, and say how long that took whether it worked or not."""
    started = time.monotonic()
    try:
        with _OPENER.open(target, timeout=2) as r:
            json.load(r)
        return _Check(time.monotonic() - started, "")
    except (urllib.error.URLError, OSError, ValueError) as e:
        return _Check(time.monotonic() - started, str(e))


def check_dependencies(page_url: str) -> None:
    """Say which of the overlay's dependencies are down, and keep going.

    Warn rather than exit, both times: `EventSource` reconnects on its own, so
    an overlay started before kotodex-server catches up when kotodex-server arrives,
    and Anki is only needed at the moment a word is mined. Exiting would make
    the start order matter when it does not.

    Textractor is deliberately absent. The page already reports it live from
    `settings.vn_logger_heartbeat`, in `#warn`, which stays right when it stops
    mid-session — a check here would be a second answer that goes stale.
    """
    origin = urlsplit(page_url)
    api = f"{origin.scheme}://{origin.netloc}"

    # Both checks are timed, because `timeout` does not bound all of what they
    # do: it applies to the socket, never to resolving the name in front of it.
    # A check that took far longer than its own timeout is the report that says
    # so, and both of these run before anything is drawn.
    took = _timed_check(f"{api}/api/settings")
    if took.error:
        print(
            f"{layer_overlay.since_start()} kotodex-server not answering on {api} "
            f"({took.error}) after {took.seconds:.2f}s — the strip stays empty "
            "until it does. Start Kotodex, which runs it.",
            file=sys.stderr,
        )
    else:
        print(f"{layer_overlay.since_start()} kotodex-server answered", file=sys.stderr)

    request = urllib.request.Request(
        ANKI_URL,
        data=json.dumps({"action": "version", "version": 6}).encode(),
        headers={"Content-Type": "application/json"},
    )
    took = _timed_check(request)
    if took.error:
        print(
            f"{layer_overlay.since_start()} Anki not answering on {ANKI_URL} "
            f"({took.error}) after {took.seconds:.2f}s — reading and lookups work, "
            "mining will fail",
            file=sys.stderr,
        )
    else:
        print(f"{layer_overlay.since_start()} Anki answered", file=sys.stderr)


def quit_kotodex() -> None:
    """Ask the launcher to quit, which is what the overlay's ✕ means.

    Not done here: the launcher owns kotodex-server and the capture daemon and
    knows which of them it *started*, which is the only thing that may be
    stopped. Written to its socket rather than run as `kotodex quit` — that
    starts a second Python and a second Qt application to send the same four
    bytes, and from a frozen build there is no script to run at all.

    A fresh `QCoreApplication` because the one the overlay ran under is gone by
    the time ✕ has been answered. Silent when nothing is listening: an overlay
    started by hand has no launcher to quit.
    """
    from PySide6.QtCore import QCoreApplication
    from PySide6.QtNetwork import QLocalSocket

    app = QCoreApplication.instance() or QCoreApplication([])
    sock = QLocalSocket()
    sock.connectToServer(LAUNCHER_SOCKET)
    if sock.waitForConnected(300):
        sock.write(b"quit")
        sock.flush()
        sock.waitForBytesWritten(300)
        sock.disconnectFromServer()
    del app


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--mobile", action="store_true", help="1.75x, with touch buttons")
    args, qt_args = ap.parse_known_args()
    scale = 1.75 if args.mobile else 1

    height = int(os.environ.get("VN_OVERLAY_HEIGHT", 300 * scale))
    url = os.environ.get("VN_OVERLAY_URL", DEFAULT_URL)
    if url == DEFAULT_URL:
        url += f"?bg={os.environ.get('VN_OVERLAY_BG', '0.82')}&h={height}&scale={scale}"
        if args.mobile:
            url += "&mobile=1"
        font = os.environ.get("VN_OVERLAY_FONT")
        if font:
            url += f"&font={quote(font)}"

    check_dependencies(url)
    code = layer_overlay.run(url, scope=SCOPE, storage=STORAGE, qt_args=qt_args)
    if code == layer_overlay.QUIT_REQUESTED:
        quit_kotodex()
        return 0
    return code


if __name__ == "__main__":
    sys.exit(main())
