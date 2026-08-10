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
# What Textractor actually sent, before any cleaning. The only place a defect in
# Textractor's own filters (repeat removal against a ruby tag) is visible at all.
# Written repr-escaped: whether a newline is in the stream is one of the questions
# it exists to answer, and normalize() has already spent it by the time text flows.
RAW_LOG = os.path.join(RUNDIR, "raw.log")
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

# The C0 controls a VN uses as script markup, minus the whitespace three. Only
# stripped from the text — see clean_line; whether a line contains them is still
# no evidence about whether it is reading.
CONTROL = re.compile(r"[\x01-\x08\x0b\x0c\x0e-\x1f]")

# A soft line break the engine emits as literal markup rather than rendering.
# Kept as a newline, not dropped: it is where the game breaks the line, so the
# overlay draws the text the shape it has on screen. Costs nothing in the count
# — count_chars is an allowlist and whitespace is outside it, while the literal
# <br> reached Sudachi and put "b" in the vocabulary ledger.
BR = re.compile(r"<br\s*/?>", re.I)

# The rest of TextMeshPro's rich text, which reaches the hook unrendered for the
# same reason <br> does. Only the tags go; the text they wrap is dialogue and
# stays. Named tags rather than any <...> run, because ASCII angle brackets do
# occur in real lines (emoticons), and a blanket rule would eat them. Left in,
# a tag costs the count as well as the ledger: its own letters are inside
# count_chars' allowlist, so <color=#9c8eff>b</color> counted 17 of the 23
# characters on that line.
RICH_TAG = re.compile(
    r"</?(?:color|b|i|u|s|em|strong|size|font|material|quad|sprite|link|align"
    r"|cspace|mspace|indent|line-height|line-indent|margin|mark|nobr|noparse"
    r"|page|pos|space|sub|sup|style|voffset|width|gradient|rotate"
    r"|allcaps|smallcaps|uppercase|lowercase)"
    r"(?:=[^<>]*)?\s*/?>",
    re.I,
)

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


# Some hooks emit every character of the line four times over. Textractor's
# own "Remove Repeated Characters" collapses runs, which is wrong twice: it flattens
# a genuine repeat (っっ, ーー, ととと) to one character, and it cannot see the shape
# furigana arrives in. What the hook repeats is a rendered *fragment*: the fragment
# comes twice with every character inside it doubled — the same four copies of each
# character, in a different order, and a one-character fragment is indistinguishable
# from a plain run of four. Furigana has no markup at all: the reading is inlined
# after the character it annotates, inside that character's fragment, so 瞠目(どうもく)
# hooks as 瞠瞠どどううももくく瞠瞠どどううももくく目目目目 and run-collapsing left half
# of it inline in the text. Undoing it here instead: a run divisible by four is that
# many characters, a stretch of doubled runs is a fragment. Anything that does not
# decode cleanly is left alone, which is what makes this safe for other hooks.
QUAD = 4
_KANJI = re.compile(r"[々〆〇㐀-䶿一-鿿豈-﫿]")
_KANA_ONLY = re.compile(r"\A[ぁ-ゟ゠-ヿ]+\Z")


def _runs(text):
    runs = []
    for c in text:
        if runs and runs[-1][0] == c:
            runs[-1][1] += 1
        else:
            runs.append([c, 1])
    return runs


# A fragment carrying furigana is one kanji followed by its reading. A fragment
# that is anything else is text: the speaker-name field is a fragment of its own,
# which is why 恵輔「ッ！」 hooks with 恵輔 doubled ahead of a plain 「ッ！」. Two
# things separate the two cases — a name written kanji-then-kana (恵ちゃん) would
# otherwise lose its kana to a reading. The reading must be two or more mora, and
# the speaker field is line-initial and followed by its opening quote.
MIN_READING = 2
_QUOTE_OPEN = "「『（(【〈《"


def _classify(token, index, tokens):
    kind, body = token
    if kind != "fragment":
        return token
    reading = body[1:]
    heads_a_quote = index == 0 and any(
        t[1].startswith(tuple(_QUOTE_OPEN)) for t in tokens[1:2]
    )
    if (
        heads_a_quote
        or len(reading) < MIN_READING
        or not _KANJI.match(body[0])
        or not _KANA_ONLY.match(reading)
    ):
        return ("name", body)
    return ("ruby", body[0], reading)


def collapse_repeats(text):
    """`text` with the hook's fourfold repetition undone and its inlined furigana
    turned into the engine ruby markup split_ruby reads, or None if `text` is not
    in that shape."""
    runs = _runs(text)
    tokens = []  # ("plain", text) | ("fragment", text)
    i = 0
    while i < len(runs):
        char, n = runs[i]
        if n % QUAD == 0:
            single = char * (n // QUAD)
            if tokens and tokens[-1][0] == "plain":
                tokens[-1] = ("plain", tokens[-1][1] + single)
            else:
                tokens.append(("plain", single))
            i += 1
            continue
        if n != 2:
            return None
        end = i
        while end < len(runs) and runs[end][1] == 2:
            end += 1
        half = (end - i) // 2
        if (end - i) % 2 or runs[i : i + half] != runs[i + half : end]:
            return None
        tokens.append(("fragment", "".join(c for c, _ in runs[i : i + half])))
        i = end
    # A short line of one repeated character (ーーーー) decodes as cleanly as a
    # quadrupled line does, and from a game that does not repeat anything it is
    # real text. Only a line long enough that the shape cannot be coincidence.
    if len(runs) < 3 or len(text) < 12:
        return None
    tokens = [_classify(t, k, tokens) for k, t in enumerate(tokens)]
    # The hook groups the reading with the *first* character of the word only, so
    # 帆刈(ほかり) arrives as a 帆ほかり fragment followed by a plain 刈. Pull the
    # following kanji back under the reading, capped at one kanji per two mora —
    # otherwise a name followed by an unrelated kanji word takes the furigana with
    # it. Wrong only in where the reading is drawn; the line itself is unaffected.
    for k, token in enumerate(tokens):
        if token[0] != "ruby":
            continue
        base, reading = token[1], token[2]
        cap = max(1, -(-len(reading) // 2))
        following = tokens[k + 1] if k + 1 < len(tokens) else None
        if following and following[0] == "plain":
            take = 0
            while (
                take < len(following[1])
                and len(base) + take < cap
                and _KANJI.match(following[1][take])
            ):
                take += 1
            if take:
                base += following[1][:take]
                tokens[k + 1] = ("plain", following[1][take:])
        tokens[k] = ("ruby", base, reading)
    return "".join(
        f"<ruby={t[2]}>{t[1]}</ruby>" if t[0] == "ruby" else t[1] for t in tokens
    )


# The speaker field, for a hook that renders it into the same string as the line
# (【speaker】 above is the other shape). The line wants what was said, not who said
# it: the name is not read, and counted it inflates every dialogue line by its own
# length. Cut only a short prefix in front of an opening quote that closes at the
# end of the line — that is a name field, while 俺は「バカ」と呼ばれた。 is a line
# with a quote in it and keeps its 俺は. Any furigana on the name goes with it, so
# the cut lands between whole ruby tags and split_ruby still lines its offsets up.
MAX_SPEAKER_CHARS = 10
_NAME_FIELD = re.compile(
    r"\A(?P<name>(?:<ruby[^<>]*>|</ruby\s*>|[^「。、！…])+?)(?P<line>「.*」)\Z", re.S
)


def strip_speaker(text):
    m = _NAME_FIELD.match(text)
    if not m:
        return text
    name = RUBY_STRAY.sub("", m.group("name"))
    return m.group("line") if 0 < len(name) <= MAX_SPEAKER_CHARS else text


def clean_line(raw):
    """Dialogue text to log for `raw`, or None to drop the capture.

    For Dohna Dohna's script-layer captures this keeps only the dialogue runs,
    strips the 【speaker】 tag, and drops skip-through captures (many lines fused
    into one). Other games carry no markers and pass through unchanged. Either
    way a capture longer than a real line, or with no Japanese left, is dropped.

    The repetition collapse runs first: at four copies of every character a normal
    line is over the length guard below and would be dropped as a skip-through.
    """
    raw = collapse_repeats(raw) or raw
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
    # Strip the markup codes, having declined to drop the line for them. They
    # are the VN's, not the reader's, and Sudachi analyses them as words —
    # \x05 and \x04 reached read-stats' vocabulary ledger as "e" and "d". No
    # effect on the count: NOT_COUNTED is an allowlist and never counted them.
    return strip_speaker(RICH_TAG.sub("", BR.sub("\n", CONTROL.sub("", text))))


# Furigana, in the two shapes a hook produces it: this engine's
# <ruby="おおごと">大事</ruby>, and the HTML one with the reading in <rt> and
# fallback parentheses in <rp>. RICH_TAG deliberately leaves all three alone so
# split_ruby can pair each reading with the text it annotates.
RUBY = re.compile(
    r"<ruby(?:\s*=\s*[\"']?(?P<attr>[^\"'<>]*)[\"']?)?\s*>(?P<body>.*?)</ruby\s*>",
    re.I | re.S,
)
RT = re.compile(r"<rt\s*>(.*?)</rt\s*>", re.I | re.S)
RP = re.compile(r"<rp\s*>.*?</rp\s*>", re.I | re.S)
# A ruby tag that never closed, so the pair above could not match it. The
# reading is dropped with the tag rather than left in the line — furigana is a
# gloss on the spelling, not part of it (see clean_field in services/anki.rs,
# which learnt this on 節穴 arriving as 節ふし穴).
RUBY_STRAY = re.compile(r"</?ruby(?:\s*=[^<>]*)?\s*>|<r[tp]\s*>.*?(?=<|$)", re.I | re.S)


def u16len(s):
    """Length in UTF-16 code units — what highlight::Span offsets count in, and
    what a JS string indexes in, so the overlay slices both the same way."""
    return len(s.encode("utf-16-le")) // 2


def split_ruby(text):
    """`(text without furigana, [[start, len, reading], ...])`.

    The reading comes out of the line and travels beside it. Left inline it
    would be counted as characters read, tokenized as part of the word, and
    written to the ledger — the spelling is 大事, never 大事おおごと.
    """
    out, spans, at = [], [], 0
    for m in RUBY.finditer(text):
        out.append(RUBY_STRAY.sub("", text[at : m.start()]))
        body = m.group("body")
        reading = m.group("attr") or " ".join(RT.findall(body))
        base = RUBY_STRAY.sub("", RT.sub("", RP.sub("", body)))
        if base and reading.strip():
            spans.append([sum(u16len(p) for p in out), u16len(base), reading.strip()])
        out.append(base)
        at = m.end()
    out.append(RUBY_STRAY.sub("", text[at:]))
    return "".join(out), spans

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
    discarded INTEGER NOT NULL DEFAULT 0,
    ruby   TEXT
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
                "ruby TEXT",
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

    def add(self, ts, text, ruby=None):
        if self.db is None:
            return
        chars = len(NOT_COUNTED.sub("", text))
        # clean_line() already dropped UI, skip-through and runaway captures, so
        # everything reaching here is real dialogue: insert not discarded. The
        # discarded column stays for the reader's manual clear button.
        self.pending.append(
            (ts, chars, text, self.current_work(), json.dumps(ruby, ensure_ascii=False) if ruby else None)
        )
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
                "INSERT INTO lines (ts, chars, text, source, work, discarded, ruby)"
                " VALUES (?, ?, ?, 'vn', ?, 0, ?)",
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
        try:
            with open(RAW_LOG, "a", encoding="utf-8") as rawlog:
                rawlog.write(f"{time.time():.9f}\t{raw!r}\n")
        except OSError:
            pass
        text = clean_line(normalize(raw))
        if not text:
            continue
        # The log and the dedup below both want the line as written: furigana
        # is a separate layer from here on, and only the overlay draws it.
        text, ruby = split_ruby(text)
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
        stats.add(ts, text, ruby)
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
