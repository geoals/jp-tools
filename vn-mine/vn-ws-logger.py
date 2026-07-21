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
    work   TEXT,
    discarded INTEGER NOT NULL DEFAULT 0
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
            for column in (
                "work TEXT",
                "discarded INTEGER NOT NULL DEFAULT 0",
            ):
                try:
                    self.db.execute(f"ALTER TABLE lines ADD COLUMN {column}")
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
            chars = len(NOT_COUNTED.sub("", text))
            # Title set via the dashboard's "now reading" field; read per line
            # so a change applies immediately without restarting the daemon.
            row = self.db.execute(
                "SELECT value FROM settings WHERE key = 'current_work'"
            ).fetchone()
            work = row[0] if row and row[0] else None
            # clean_line() already dropped UI, skip-through and runaway captures,
            # so everything reaching here is real dialogue: insert not discarded.
            # The discarded column stays for the reader's manual clear button.
            self.db.execute(
                "INSERT INTO lines (ts, chars, text, source, work, discarded)"
                " VALUES (?, ?, ?, 'vn', ?, 0)",
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
