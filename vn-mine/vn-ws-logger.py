#!/usr/bin/env python3
"""VN mine hooked-line logger.

Connects to a Textractor WebSocket server (the same ws:// endpoint the
texthooker-ui reads) and appends every hooked Japanese line to lines.log as
"<epoch>\t<text>", one per line. vn-capture.sh anchors the last voiceline on
the newest entry's timestamp.

Replaces the old clipboard watcher: the WS stream carries only Textractor
hooks, so copying a sentence for a lookup/card no longer pollutes the log and
there is no startup clipboard replay to guard against.

Every logged line is also inserted into the shared knowledge SQLite DB
(durable, unlike the tmpfs lines.log) so reading time and character counts can
be derived without any manual tracking. Stats failures never block mining.

Two databases are involved, and the split is jp-core's: `lines` is knowledge,
shared with every tool that asks
what has been read, while `settings.current_work` — the title stamped on each
line — is read-stats' own. The knowledge DB is the connection; read-stats' is
attached read-only-in-practice for two settings: `current_work` and
`capture_paused`.

Pausing is a *source* stop, not a filter. While `settings.capture_paused` is
set, this closes the Textractor connection and stays disconnected, so nothing
enters the line stream at all. The close is a proper close frame — the same
path SIGTERM takes — because an abortive disconnect crashes Textractor's WS
plugin and takes Textractor with it.

Env:
  VN_RUNDIR                   run dir (default: $XDG_RUNTIME_DIR/vn-mine or /run/user/$UID/...)
  VN_WS_URL                   WebSocket URL (default: ws://localhost:6677)
  JP_TOOLS_KNOWLEDGE_DB_PATH  shared knowledge DB (default: ~/.local/share/jp-tools/knowledge.db)
  JP_TOOLS_STATS_DB_PATH      read-stats DB (default: ~/.local/share/jp-tools/read-stats.db)
  JP_TOOLS_STATS_DISABLE      set to 1 to skip the stats sink entirely
"""
import asyncio
import json
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

# How often the capture-paused flag is re-read, and how long to wait before
# retrying a connection that failed. Both are cheap: one indexed SQLite read
# and one localhost socket.
PAUSE_POLL_SECS = 2.0
RECONNECT_SECS = 2.0

# Heartbeat cadence. `#read` calls the logger down at three missed beats, so
# this also sets how fast a dead logger is noticed.
HEARTBEAT_SECS = 2.0

# only Japanese text marks a voiceline; ignore stray latin/punctuation hooks
JP = re.compile(r"[぀-ヿ一-鿿]")

# Character counting matches texthooker-ui's isNotJapaneseRegex (an allowlist,
# so punctuation and brackets don't count) — otherwise read-stats reports a
# chars/h noticeably above what the texthooker shows for the same reading.
# Keep in sync with read-stats/src/charcount.rs.
_COUNTED = (
    "0-9A-Za-z"
    "○◯"  # ○ ◯
    "々-〇〻"  # 々 〆 〇 〻
    "ぁ-ゖゝ-ゞ"  # ぁ-ゖ ゝ ゞ
    "ァ-ヺー"  # ァ-ヺ ー
    "０-９Ａ-Ｚａ-ｚ"  # ０-９ Ａ-Ｚ ａ-ｚ
    "ｦ-ﾝ"  # ｦ-ﾝ halfwidth katakana
    "⺀-⺙⺛-⻳⼀-⿕"  # \p{Radical}
    "㐀-䶿一-鿿"  # \p{Unified_Ideograph}
    "﨎-﨏﨑﨓-﨔﨟﨡﨣-﨤﨧-﨩"
    "\U00020000-\U0002a6df\U0002a700-\U0002b81d\U0002b820-\U0002cead"
    "\U0002ceb0-\U0002ebe0\U0002ebf0-\U0002ee5d"
    "\U00030000-\U0003134a\U00031350-\U00033479"
)
NOT_COUNTED = re.compile(f"[^{_COUNTED}]")

# A hook pointed at the wrong address dumps whole memory regions instead of
# dialogue, and Dohna Dohna's script-layer hook fuses many lines into one
# capture while skip is held. Either way a capture far longer than a real line
# is not reading: it is dropped, not logged and not counted. The raw WS stream
# is gone once dropped, but lines.log/the DB were never meant to be a verbatim
# mirror of it. Real lines observed top out around 90 chars; the guard sits well
# above that. Deliberately NOT filtering on control characters — VNs use them as
# text markup (Subahibi puts \x05 at the head of narration lines and \x04
# mid-clause), so presence of them says nothing about whether a line is real
# reading.
MAX_READING_CHARS = 500

# Dohna Dohna (Alicesoft System 4.3), hook HS932#-C@289F60:main.bin, taps the
# script-text layer before rendering, so one capture interleaves dialogue with
# UI/animation directives. The two are self-labelling: the engine's own regexes
# arrive verbatim in the stream ahead of the strings they process — a literal
# ${...} markup-strip pattern heads each dialogue run, a literal [...]-section
# pattern heads each block of menu/animation/widget junk (Section:…, [X:…],
# Button\d…, enemy names). Split on those two literals and keep only what a
# dialogue marker introduced. Captures from any other game carry neither literal
# and pass through untouched.
_DIALOG_MARK = r"\$\{[^\}]+\}"
_UI_MARK = r"([^\[\]]+?)+|\[[^\]]+?\]"
_SEGMENT = re.compile("(" + re.escape(_DIALOG_MARK) + "|" + re.escape(_UI_MARK) + ")")
# Each fused line is headed by its 【speaker】 tag; holding skip fuses a crowd of
# them into one capture. Normal reading tops out at ~4 tags per capture, a skip
# burst runs 20+, so a tag count this high means skipping, not reading — drop it.
# The tag is also stripped from what survives: the card wants the line, not who.
_SPEAKER = re.compile(r"【[^】]*】")
MAX_SPEAKER_TAGS = 5


def clean_line(raw):
    """Dialogue text to log for `raw`, or None to drop the capture.

    For Dohna Dohna's script-layer captures this keeps only the dialogue runs,
    strips the 【speaker】 tag, and drops skip-through captures (many lines fused
    into one). Other games carry no markers and pass through unchanged. Either
    way a capture longer than a real line, or with no Japanese left, is dropped.
    """
    parts = _SEGMENT.split(raw)
    if len(parts) > 1:  # Dohna Dohna script-layer capture
        runs, keep = [], False
        for part in parts:
            if part == _DIALOG_MARK:
                keep = True
            elif part == _UI_MARK:
                keep = False
            elif keep and part:
                runs.append(part)
        text = "".join(runs)
        if len(_SPEAKER.findall(text)) >= MAX_SPEAKER_TAGS:
            return None  # skip-through: a crowd of lines fused into one capture
        text = _SPEAKER.sub("", text).strip()
    else:
        text = raw
    # Textractor hands us already-decoded text, so a backslash never occurs in
    # real dialogue. It does occur in Dohna Dohna's widget-registry dumps, which
    # reach here marker-less (Button\dText2Button\dルートパーツ…) and would
    # otherwise slip through on their stray katakana. One rule catches them all.
    if "\\" in text:
        return None
    if len(text) > MAX_READING_CHARS or not JP.search(text):
        return None
    return text

KNOWLEDGE_DB = os.environ.get("JP_TOOLS_KNOWLEDGE_DB_PATH") or os.path.expanduser(
    "~/.local/share/jp-tools/knowledge.db"
)
STATS_DB = os.environ.get("JP_TOOLS_STATS_DB_PATH") or os.path.expanduser(
    "~/.local/share/jp-tools/read-stats.db"
)

# Keep in sync with jp-core/migrations/knowledge/004_reading.sql — whichever
# process starts first creates the schema.
KNOWLEDGE_SCHEMA = """
CREATE TABLE IF NOT EXISTS lines (
    id     INTEGER PRIMARY KEY,
    ts     REAL    NOT NULL,
    chars  INTEGER NOT NULL,
    text   TEXT,
    source TEXT    NOT NULL DEFAULT 'vn',
    work   TEXT,
    discarded INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_lines_ts ON lines(ts);
"""

# read-stats/migrations/001_settings_and_pauses.sql. Created here only so the
# current_work lookup has something to read on a first-ever run; read-stats
# owns the table and everything else in that file.
STATS_SCHEMA = """
CREATE TABLE IF NOT EXISTS stats.settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"""


class StatsSink:
    """Best-effort writer into the read-stats DB; never interferes with mining.

    A write that fails goes to `pending` and is retried on the next line rather
    than dropped. The failure this exists for is a batch job elsewhere holding
    the single SQLite write lock for longer than `busy_timeout`: transient, but
    it once ran the whole session's lines into the ground because the sink gave
    up permanently after a handful of errors.
    """

    MAX_PENDING = 2000

    def __init__(self):
        self.db = None
        self.pending = []
        if os.environ.get("JP_TOOLS_STATS_DISABLE"):
            return
        try:
            os.makedirs(os.path.dirname(KNOWLEDGE_DB), exist_ok=True)
            os.makedirs(os.path.dirname(STATS_DB), exist_ok=True)
            self.db = sqlite3.connect(KNOWLEDGE_DB, isolation_level=None)
            self.db.execute("PRAGMA journal_mode=WAL")
            self.db.execute("PRAGMA busy_timeout=5000")
            self.db.executescript(KNOWLEDGE_SCHEMA)
            # read-stats' own DB, for the current_work and capture_paused
            # settings only.
            self.db.execute("ATTACH DATABASE ? AS stats", (STATS_DB,))
            self.db.executescript(STATS_SCHEMA)
            for column in (
                "work TEXT",
                "discarded INTEGER NOT NULL DEFAULT 0",
            ):
                try:
                    self.db.execute(f"ALTER TABLE lines ADD COLUMN {column}")
                except sqlite3.OperationalError:
                    pass  # column already exists
            log(f"stats sink: {KNOWLEDGE_DB} (+ settings from {STATS_DB})")
        except (OSError, sqlite3.Error) as e:
            log(f"stats sink unavailable ({e}) — reading stats disabled")
            self.db = None

    def heartbeat(self, ws_up):
        """Publish what only this process knows: that it is alive, whether
        Textractor is actually attached, and whether its writes are landing.

        `#read`'s live badge is otherwise the browser's own SSE connection,
        which stays perfectly healthy while nothing at all is being captured.
        """
        if self.db is None:
            return
        beat = {"ts": time.time(), "ws": bool(ws_up), "pending": len(self.pending)}
        try:
            self.db.execute(
                "INSERT INTO stats.settings (key, value)"
                " VALUES ('vn_logger_heartbeat', ?)"
                " ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                (json.dumps(beat),),
            )
        except sqlite3.Error:
            pass  # a heartbeat that cannot be written is itself the outage

    def capture_paused(self):
        """Whether the dashboard has capture switched off.

        Fails *open* — an unreadable flag means keep capturing. Losing lines to
        a database hiccup would be silent and unrecoverable; capturing a few
        that should have been paused is visible and clearable from the reader.
        """
        if self.db is None:
            return False
        try:
            row = self.db.execute(
                "SELECT value FROM stats.settings WHERE key = 'capture_paused'"
            ).fetchone()
            return bool(row) and row[0] == "1"
        except sqlite3.Error as e:
            log(f"pause flag unreadable ({e}) — continuing to capture")
            return False

    def current_work(self):
        """Title set via the dashboard's "now reading" field; read per line so a
        change applies immediately without restarting the daemon."""
        try:
            row = self.db.execute(
                "SELECT value FROM stats.settings WHERE key = 'current_work'"
            ).fetchone()
            return row[0] if row and row[0] else None
        except sqlite3.Error:
            return None

    def add(self, ts, text):
        if self.db is None:
            return
        chars = len(NOT_COUNTED.sub("", text))
        # clean_line() already dropped UI, skip-through and runaway captures, so
        # everything reaching here is real dialogue: insert not discarded. The
        # discarded column stays for the reader's manual clear button.
        self.pending.append((ts, chars, text, self.current_work()))
        # Under a lock held for minutes this keeps the newest lines; the oldest
        # are the ones already in lines.log the longest, so they stay
        # recoverable from there.
        del self.pending[: -self.MAX_PENDING]
        self.flush()

    def flush(self):
        if not self.pending:
            return
        stuck = len(self.pending) > 1
        try:
            self.db.execute("BEGIN IMMEDIATE")
            self.db.executemany(
                "INSERT INTO lines (ts, chars, text, source, work, discarded)"
                " VALUES (?, ?, ?, 'vn', ?, 0)",
                self.pending,
            )
            self.db.execute("COMMIT")
        except sqlite3.Error as e:
            if self.db.in_transaction:
                self.db.rollback()
            if not stuck:
                log(f"stats insert failed ({e}) — holding lines for retry")
            return
        if stuck:
            log(f"stats sink recovered, wrote {len(self.pending)} held lines")
        self.pending.clear()


def log(msg):
    print(f"vn-ws-logger: {msg}", file=sys.stderr, flush=True)


def normalize(msg):
    text = msg.replace("\r", " ").replace("\n", " ").strip()
    return text[:4000]


async def read_lines(ws, out, stats, last_text):
    """Drain one connection into the log and the stats sink."""
    async for raw in ws:
        if isinstance(raw, bytes):
            raw = raw.decode("utf-8", "replace")
        text = clean_line(normalize(raw))
        if not text:
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
    return last_text


async def watch_pause(ws, stats):
    """Close the connection cleanly once capture is paused.

    A separate task because the read loop is parked in `async for`: this is what
    turns the flag into an actual disconnect rather than a filter.
    """
    while True:
        await asyncio.sleep(PAUSE_POLL_SECS)
        if stats.capture_paused():
            log("capture paused — closing the Textractor connection")
            await ws.close()
            return


async def pump(out, stats, state):
    last_text = None
    # An explicit loop rather than `async for ws in websockets.connect(...)`:
    # that form reconnects on its own, which is exactly what a pause must not
    # do. Reconnecting is now a decision made here, once per iteration, after
    # the flag has been checked.
    while True:
        if stats.capture_paused():
            await asyncio.sleep(PAUSE_POLL_SECS)
            continue
        try:
            async with websockets.connect(
                WS_URL, max_size=None, ping_interval=20, ping_timeout=20
            ) as ws:
                log(f"connected to {WS_URL}")
                state["ws"] = ws
                watcher = asyncio.create_task(watch_pause(ws, stats))
                try:
                    last_text = await read_lines(ws, out, stats, last_text)
                except websockets.ConnectionClosed:
                    log("connection closed")
                finally:
                    watcher.cancel()
                    state["ws"] = None
        except (OSError, websockets.WebSocketException) as e:
            log(f"connect failed ({e}), retrying")
            await asyncio.sleep(RECONNECT_SECS)


async def beat(stats, state):
    """Heartbeat, and the retry that does not depend on a next line arriving —
    a sink that failed on the session's last line would otherwise hold it until
    reading resumed."""
    while True:
        stats.flush()
        stats.heartbeat(state["ws"] is not None)
        await asyncio.sleep(HEARTBEAT_SECS)


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
    beat_task = asyncio.create_task(beat(stats, state))
    stop_task = asyncio.create_task(stop.wait())
    await asyncio.wait({pump_task, stop_task}, return_when=asyncio.FIRST_COMPLETED)

    for task in (pump_task, beat_task):
        task.cancel()
        try:
            await task
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
