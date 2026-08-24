#!/usr/bin/env python3
"""The line being read, as a strip over the VN, fullscreen included.

The page is `overlay.html` beside this file — the newest line, and a dictionary
popup for the word clicked in it. Yomitan does not run here (QtWebEngine loads
no extensions), so the popup is the page's own.

read-stats is the backend, and serves the page from this directory: it calls
`/api` for the line stream, the dictionary, the ledger and the card, and none of
that can be answered locally. Starting without read-stats gets a warning, not a
failure — `EventSource` reconnects on its own, so the strip fills in when
read-stats arrives.

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

    VN_OVERLAY_URL      page to show      (default overlay.html, over read-stats)
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
import urllib.error
import urllib.request
from pathlib import Path
from urllib.parse import quote, urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "layer-overlay"))
import layer_overlay  # noqa: E402

DEFAULT_URL = "http://localhost:3200/overlay/overlay.html"
ANKI_URL = os.environ.get("KOTODEX_ANKI_URL", "http://127.0.0.1:8765")
# Names the surface to the compositor and the page's localStorage. Unchanged
# from when the shell lived in vn-mine, so the type settings, ghost mode and
# both drag offsets survived the move.
SCOPE = "vn-overlay"
STORAGE = (
    Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share") / "vn-mine/overlay"
)


def check_dependencies(page_url: str) -> None:
    """Say which of the overlay's dependencies are down, and keep going.

    Warn rather than exit, both times: `EventSource` reconnects on its own, so
    an overlay started before read-stats catches up when read-stats arrives,
    and Anki is only needed at the moment a word is mined. Exiting would make
    the start order matter when it does not.

    Textractor is deliberately absent. The page already reports it live from
    `settings.vn_logger_heartbeat`, in `#warn`, which stays right when it stops
    mid-session — a check here would be a second answer that goes stale.
    """
    origin = urlsplit(page_url)
    api = f"{origin.scheme}://{origin.netloc}"

    try:
        with urllib.request.urlopen(f"{api}/api/settings", timeout=2) as r:
            json.load(r)
    except (urllib.error.URLError, OSError, ValueError) as e:
        print(
            f"read-stats not answering on {api} ({e}) — the strip stays empty "
            "until it does. scripts/start-all.sh start read-stats",
            file=sys.stderr,
        )

    request = urllib.request.Request(
        ANKI_URL,
        data=json.dumps({"action": "version", "version": 6}).encode(),
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=2) as r:
            json.load(r)
    except (urllib.error.URLError, OSError, ValueError) as e:
        print(
            f"Anki not answering on {ANKI_URL} ({e}) — reading and lookups work, "
            "mining will fail",
            file=sys.stderr,
        )


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
    return layer_overlay.run(url, scope=SCOPE, storage=STORAGE, qt_args=qt_args)


if __name__ == "__main__":
    sys.exit(main())
