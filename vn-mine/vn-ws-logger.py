#!/usr/bin/env python3
"""VN mine hooked-line logger.

Connects to a Textractor WebSocket server (the same ws:// endpoint the
texthooker-ui reads) and appends every hooked Japanese line to lines.log as
"<epoch>\t<text>", one per line. vn-capture.sh anchors the last voiceline on
the newest entry's timestamp.

Replaces the old clipboard watcher: the WS stream carries only Textractor
hooks, so copying a sentence for a lookup/card no longer pollutes the log and
there is no startup clipboard replay to guard against.

Every logged line is also inserted into the read-stats SQLite DB (durable,
unlike the tmpfs lines.log) so reading time and character counts can be
derived without any manual tracking. Stats failures never block mining.

Env:
  VN_RUNDIR               run dir (default: $XDG_RUNTIME_DIR/vn-mine or /run/user/$UID/...)
  VN_WS_URL               WebSocket URL (default: ws://localhost:6677)
  JP_TOOLS_STATS_DB_PATH  read-stats DB (default: ~/.local/share/jp-stats/stats.db)
  JP_TOOLS_STATS_DISABLE  set to 1 to skip the stats sink entirely
"""
import asyncio
import os
import re
import signal
import sqlite3
import sys
import time

import websockets

RUNDIR = os.environ.get("VN_RUNDIR") or os.path.join(
    os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.getuid()}", "vn-mine"
)
LINES_LOG = os.path.join(RUNDIR, "lines.log")
WS_URL = os.environ.get("VN_WS_URL", "ws://localhost:6677")

# only Japanese text marks a voiceline; ignore stray latin/punctuation hooks
JP = re.compile(r"[぀-ヿ一-鿿]")

STATS_DB = os.environ.get("JP_TOOLS_STATS_DB_PATH") or os.path.expanduser(
    "~/.local/share/jp-stats/stats.db"
)

# Keep in sync with read-stats/migrations/001_create_stats_tables.sql —
# whichever process starts first creates the schema.
STATS_SCHEMA = """
CREATE TABLE IF NOT EXISTS lines (
    id     INTEGER PRIMARY KEY,
    ts     REAL    NOT NULL,
    chars  INTEGER NOT NULL,
    text   TEXT,
    source TEXT    NOT NULL DEFAULT 'vn',
    work   TEXT
);
CREATE INDEX IF NOT EXISTS idx_lines_ts ON lines(ts);
CREATE TABLE IF NOT EXISTS sessions (
    id       INTEGER PRIMARY KEY,
    start_ts REAL    NOT NULL,
    end_ts   REAL    NOT NULL,
    chars    INTEGER NOT NULL,
    source   TEXT    NOT NULL DEFAULT 'book',
    work     TEXT,
    pages    REAL,
    note     TEXT
);
CREATE INDEX IF NOT EXISTS idx_sessions_start_ts ON sessions(start_ts);
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS pauses (
    id       INTEGER PRIMARY KEY,
    start_ts REAL NOT NULL,
    end_ts   REAL
);
"""


class StatsSink:
    """Best-effort writer into the read-stats DB; disables itself after
    repeated errors rather than ever interfering with mining."""

    MAX_ERRORS = 5

    def __init__(self):
        self.db = None
        self.errors = 0
        if os.environ.get("JP_TOOLS_STATS_DISABLE"):
            return
        try:
            os.makedirs(os.path.dirname(STATS_DB), exist_ok=True)
            self.db = sqlite3.connect(STATS_DB, isolation_level=None)
            self.db.execute("PRAGMA journal_mode=WAL")
            self.db.execute("PRAGMA busy_timeout=5000")
            self.db.executescript(STATS_SCHEMA)
            try:
                self.db.execute("ALTER TABLE lines ADD COLUMN work TEXT")
            except sqlite3.OperationalError:
                pass  # column already exists
            log(f"stats sink: {STATS_DB}")
        except (OSError, sqlite3.Error) as e:
            log(f"stats sink unavailable ({e}) — reading stats disabled")
            self.db = None

    def add(self, ts, text):
        if self.db is None:
            return
        try:
            chars = len(re.sub(r"\s", "", text))
            # Title set via the dashboard's "now reading" field; read per line
            # so a change applies immediately without restarting the daemon.
            row = self.db.execute(
                "SELECT value FROM settings WHERE key = 'current_work'"
            ).fetchone()
            work = row[0] if row and row[0] else None
            self.db.execute(
                "INSERT INTO lines (ts, chars, text, source, work) VALUES (?, ?, ?, 'vn', ?)",
                (ts, chars, text, work),
            )
            self.errors = 0
        except sqlite3.Error as e:
            self.errors += 1
            log(f"stats insert failed ({e})")
            if self.errors >= self.MAX_ERRORS:
                log("stats sink disabled after repeated errors")
                self.db = None


def log(msg):
    print(f"vn-ws-logger: {msg}", file=sys.stderr, flush=True)


def normalize(msg):
    text = msg.replace("\r", " ").replace("\n", " ").strip()
    return text[:4000]


async def pump(out, stats, state):
    last_text = None
    async for msg in websockets.connect(
        WS_URL, max_size=None, ping_interval=20, ping_timeout=20
    ):
        log(f"connected to {WS_URL}")
        state["ws"] = msg
        try:
            async for raw in msg:
                if isinstance(raw, bytes):
                    raw = raw.decode("utf-8", "replace")
                text = normalize(raw)
                if not text or not JP.search(text):
                    continue
                # A re-hook of the line still on screen (Textractor double-fire,
                # focus change) must not move the anchor. Only the immediately
                # preceding line is suppressed, so a genuine later repeat of the
                # same short line — separated by other dialogue — still logs.
                if text == last_text:
                    continue
                ts = time.time()
                out.write(f"{ts:.9f}\t{text}\n")
                out.flush()
                stats.add(ts, text)
                last_text = text
        except websockets.ConnectionClosed:
            log("connection closed, reconnecting")
        finally:
            state["ws"] = None
        # websockets.connect(...) as an async iterator auto-reconnects with
        # backoff on the next loop iteration.


async def run(out, stats):
    # On SIGTERM/SIGINT, send the server a proper close frame before exiting:
    # an abortive disconnect (plain process kill) can crash Textractor's
    # WebSocket plugin, taking Textractor down with it.
    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(sig, stop.set)

    state = {"ws": None}
    pump_task = asyncio.create_task(pump(out, stats, state))
    stop_task = asyncio.create_task(stop.wait())
    await asyncio.wait({pump_task, stop_task}, return_when=asyncio.FIRST_COMPLETED)

    pump_task.cancel()
    try:
        await pump_task
    except asyncio.CancelledError:
        pass
    ws = state["ws"]
    if ws is not None:
        try:
            await ws.close()
            log("closed connection cleanly")
        except Exception as e:
            log(f"close failed: {e}")


def main():
    os.makedirs(RUNDIR, exist_ok=True)
    stats = StatsSink()
    with open(LINES_LOG, "a", buffering=1, encoding="utf-8") as out:
        try:
            asyncio.run(run(out, stats))
        except KeyboardInterrupt:
            pass


if __name__ == "__main__":
    main()
