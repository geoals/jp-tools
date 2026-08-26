#!/usr/bin/env python3
"""VN mine hooked-line logger.

Connects to a Textractor WebSocket server (the same ws:// endpoint the
texthooker-ui reads) and appends every hooked Japanese line to lines.log as
"<epoch>\t<text>", one per line. vn-capture.sh anchors the last voiceline on
the newest entry's timestamp.

Two sources, one producer. `settings.line_source` picks between the WebSocket
and a clipboard watcher, and switching it takes effect without a restart — the
same way pausing does. It is a switch rather than a second script because
everything after the source is the part that matters: `clean_line`, the ruby
split, the dedup and both sinks. A second writer of `lines` would be a second
copy of all of it.

The WebSocket is the default. A clipboard hooker copies whatever the reader
copies too, so a sentence copied for a lookup enters the log as if it were
dialogue, and whatever was already on the clipboard at startup would replay —
guarded below, but not fixable in general.

Every logged line is also posted to kotodex-server's ingest endpoint
(durable, unlike the tmpfs lines.log) so reading time and character counts can
be derived without any manual tracking. Ledger failures never block mining.

This is one source among several, and it owns only what is specific to
Textractor: the hooker's junk, a continuation split across two text boxes, the
dedup. The character count, which work a line belongs to and whether capture is
paused at all are the ledger's, answered by the server — so a second source
cannot arrive at a different number for the same reading.

Pausing is a *source* stop, not a filter. While `settings.capture_paused` is
set, this closes the Textractor connection and stays disconnected, so nothing
enters the line stream at all. The close is a proper close frame — the same
path SIGTERM takes — because an abortive disconnect crashes Textractor's WS
plugin and takes Textractor with it.

Env:
  VN_RUNDIR                   run dir (default: $XDG_RUNTIME_DIR/kotodex or /run/user/$UID/...)
  VN_WS_URL                   WebSocket URL, overriding settings.line_source_ws_url
  KOTODEX_SERVER_URL          kotodex-server (default: http://127.0.0.1:3200)
  KOTODEX_INGEST_DISABLE       set to 1 to skip the ledger entirely
"""
import asyncio
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.request

import websockets

RUNDIR = os.environ.get("VN_RUNDIR") or os.path.join(
    os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.getuid()}", "kotodex"
)
LINES_LOG = os.path.join(RUNDIR, "lines.log")
# What Textractor actually sent, before any cleaning. The only place a defect in
# Textractor's own filters (repeat removal against a ruby tag) is visible at all.
# Written repr-escaped: whether a newline is in the stream is one of the questions
# it exists to answer, and normalize() has already spent it by the time text flows.
RAW_LOG = os.path.join(RUNDIR, "raw.log")
# The environment wins over the setting: a hand-started logger pointed somewhere
# for a one-off should not be moved by whatever the panel last saved.
WS_URL_ENV = os.environ.get("VN_WS_URL")
WS_URL_FALLBACK = "ws://localhost:6677"

# How often the capture-paused flag is re-read, and how long to wait before
# retrying a connection that failed. Both are cheap: one indexed SQLite read
# and one localhost socket.
PAUSE_POLL_SECS = 2.0
RECONNECT_SECS = 2.0

# How often the clipboard is read. A clipboard hooker writes on line advance, so
# this is the latency between the game's line and the overlay's — low enough not
# to be felt, and one short-lived process per tick is the whole cost.
CLIPBOARD_POLL_SECS = 0.25

# Heartbeat cadence. `#read` calls the logger down at three missed beats, so
# this also sets how fast a dead logger is noticed.
HEARTBEAT_SECS = 2.0

# only Japanese text marks a voiceline; ignore stray latin/punctuation hooks
JP = re.compile(r"[぀-ヿ一-鿿]")

# A line the script writes as nothing but punctuation — 「……」, 「──」, 「!?」 —
# is a real line with a voiceline behind it, so it is kept even though JP finds
# nothing in it. It counts zero characters (NOT_COUNTED is an allowlist), which
# is what keeps it out of every rate. The set is closed on purpose: anything
# else with no Japanese in it is a hook pointed at the wrong address.
PUNCT_ONLY = re.compile(
    r"^[\s。、，．・…‥！？!?～〜ー―‐–—─━〝〟“”\"'‘’「」『』（）()〈〉《》【】〔〕［］\[\]｛｝{}♪♡★☆※→←↑↓]+$"
)

# Character counting matches texthooker-ui's isNotJapaneseRegex (an allowlist,
# so punctuation and brackets don't count) — otherwise kotodex-server reports a
# chars/h noticeably above what the texthooker shows for the same reading.
# Keep in sync with kotodex-server/src/charcount.rs.
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
    r"\A(?P<name>(?:<ruby[^<>]*>|</ruby\s*>|[^「」。、！…])+?)(?P<line>「.*」)\Z", re.S
)


def strip_speaker(text):
    m = _NAME_FIELD.match(text)
    if not m:
        return text
    name = RUBY_STRAY.sub("", m.group("name"))
    return m.group("line") if 0 < len(name) <= MAX_SPEAKER_CHARS else text


# A hook that taps the script before the engine reads it hands the script's own
# escapes through undecoded: \n where the game breaks the line, \cd heading each
# dialogue line (clear the textbox, then continue), and \@ closing one (wait for
# the click). Decoded rather than dropped by the backslash rule in clean_line —
# which still catches every other backslash, so a widget dump has nothing to
# slip through on. Only these three, because a command that means something else
# must not be silently swallowed.
SCRIPT_ESCAPE = re.compile(r"\\(n|cd|@)")

# Text colour, which the same hook writes as \c0xRRGGBB; and \cd0xRRGGBB; — a
# narration line arrives coloured grey and every one of them was dropped by the
# backslash rule. Stripped before SCRIPT_ESCAPE, or \cd matches the head of
# \cd0xff898989; and leaves a literal 0xff898989; in the line.
COLOR_ESCAPE = re.compile(r"\\cd?0x[0-9a-fA-F]+;")

# Furigana as the same hook writes it: [眸/ひとみ]. Rewritten into the engine
# ruby markup so split_ruby pairs the reading with its text like every other
# shape, instead of the brackets reaching Sudachi and the character count.
BRACKET_RUBY = re.compile(r"\[([^\[\]/\n]+)/([^\[\]/\n]+)\]")


def _escape(m):
    return "\n" if m.group(1) == "n" else ""


# Textractor sometimes flushes one script line as two captures, split at the
# script's own \n, so the second arrives opening on the break. \cd is what clears
# the textbox, so a capture without one that opens on \n is continuing text still
# on screen and belongs to the line before it — which the overlay draws alone, so
# unmerged the first half shows for the 30ms until the rest lands. Content and
# not timing: real consecutive lines arrive 49ms apart in the same session, so no
# flush delay can separate the two cases.
CONTINUATION = re.compile(r"\A\s*\\n")


def continues_previous(raw):
    return bool(CONTINUATION.match(raw)) and "\\cd" not in raw


# Dropping a capture for an unknown backslash command is fail-closed and was
# silent: a colour code cost real narration lines twice before anyone noticed.
# Once per command per run — a choice menu re-hooks on every cursor move.
UNKNOWN_COMMAND = re.compile(r"\\[A-Za-z@]+")
_SEEN_COMMANDS = set()


def note_unknown_command(text):
    for command in UNKNOWN_COMMAND.findall(text):
        if command not in _SEEN_COMMANDS:
            _SEEN_COMMANDS.add(command)
            log(f"unknown script command {command} — those captures are dropped")


def clean_line(raw):
    """Dialogue text to log for `raw`, or None to drop the capture.

    For Dohna Dohna's script-layer captures this keeps only the dialogue runs,
    strips the 【speaker】 tag, and drops skip-through captures (many lines fused
    into one). Other games carry no markers and pass through unchanged. Either
    way a capture longer than a real line is dropped, as is one with no Japanese
    left that is not punctuation alone (see PUNCT_ONLY).

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
    text = BRACKET_RUBY.sub(
        r"<ruby=\2>\1</ruby>", SCRIPT_ESCAPE.sub(_escape, COLOR_ESCAPE.sub("", text))
    )
    # Textractor hands us already-decoded text, so a backslash never occurs in
    # real dialogue. It does occur in Dohna Dohna's widget-registry dumps, which
    # reach here marker-less (Button\dText2Button\dルートパーツ…) and would
    # otherwise slip through on their stray katakana. One rule catches them all.
    if "\\" in text:
        note_unknown_command(text)
        return None
    if len(text) > MAX_READING_CHARS:
        return None
    if not JP.search(text):
        bare = CONTROL.sub("", RICH_TAG.sub("", text)).strip()
        if not bare or not PUNCT_ONLY.match(bare):
            return None
    # Strip the markup codes, having declined to drop the line for them. They
    # are the VN's, not the reader's, and Sudachi analyses them as words —
    # \x05 and \x04 reached kotodex-server's vocabulary ledger as "e" and "d". No
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

SERVER_URL = (
    os.environ.get("KOTODEX_SERVER_URL") or "http://127.0.0.1:3200"
).rstrip("/")

# This source's name in the `lines` table, and what a retract addresses.
SOURCE = "vn"

# How long a request to the server may take before the capture loop gives up on
# it. Short because the call is synchronous: a line is held and retried rather
# than waited on, and the reading feed's latency is the point of the pipeline.
HTTP_TIMEOUT = 2.0

# How long to wait before trying the server again after it has refused a
# connection. A first-ever run has to sit out whatever starts the server.
RETRY_SECS = 30

# How stale the cached settings may get. Pause, the chosen source and the
# WebSocket address are all read from this rather than per poll: the pause loop
# asks several times a second and none of those answers changes that fast.
SETTINGS_TTL = 2.0


def get_json(path):
    """GET and decode. Raises OSError or ValueError if the server did not answer."""
    with urllib.request.urlopen(f"{SERVER_URL}{path}", timeout=HTTP_TIMEOUT) as resp:
        return json.loads(resp.read().decode("utf-8"))


def post_json(path, payload):
    """POST and decode. Raises OSError or ValueError if the server did not take it."""
    req = urllib.request.Request(
        f"{SERVER_URL}{path}",
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=HTTP_TIMEOUT) as resp:
        return json.loads(resp.read().decode("utf-8"))


class StatsSink:
    """Best-effort client for the ledger's ingest endpoint.

    HTTP rather than SQLite: the schema belongs to jp-core's migrations, and a
    writer that knows the column list is a copy of it that goes stale silently.
    It also means this daemon does not have to be on the machine holding the
    database.

    A request that fails goes to `pending` and is retried on the next line
    rather than dropped. The failure this exists for is the server being
    restarted mid-session: transient, but it once ran a whole sitting's lines
    into the ground because the sink gave up permanently after a handful of
    errors.
    """

    MAX_PENDING = 2000

    def __init__(self):
        self.pending = []
        self.disabled = bool(os.environ.get("KOTODEX_INGEST_DISABLE"))
        self.attached = False
        self._settings = {}
        self._settings_at = 0.0
        self._next_try = 0.0
        self._complained = False

    def ready(self):
        """Whether it is worth making a request at all.

        There is no connection to open, so this is only the backoff: after a
        refusal, stay quiet for `RETRY_SECS` instead of a failed request per
        captured line.
        """
        return not self.disabled and time.time() >= self._next_try

    def _unreachable(self, e, what):
        self._next_try = time.time() + RETRY_SECS
        if not self._complained:
            self._complained = True
            log(f"{what} ({e}) — lines.log still has everything")

    def _reached(self):
        self._next_try = 0.0
        if self._complained:
            self._complained = False
            log(f"ledger reachable again at {SERVER_URL}")

    def settings(self):
        """The reader's settings, cached for `SETTINGS_TTL`.

        Stale settings fail *open*: an unreachable server reads as not paused,
        on the same rule as before — losing lines is silent and unrecoverable,
        capturing a few that should have been paused is visible and clearable
        from the reader.
        """
        if self.disabled:
            return {}
        now = time.time()
        if self._settings and now - self._settings_at < SETTINGS_TTL:
            return self._settings
        if now < self._next_try:
            return self._settings
        try:
            self._settings = get_json("/api/settings")
            self._settings_at = now
            self._reached()
        except (OSError, ValueError) as e:
            self._unreachable(e, "ledger settings unreadable")
        return self._settings

    def capture_paused(self):
        return bool(self.settings().get("capture_paused"))

    def line_source(self):
        """`ws` or `clipboard`. Anything else is a setting written by hand and
        is read as the default rather than as no source at all."""
        source = self.settings().get("line_source") or "ws"
        return source if source in ("ws", "clipboard") else "ws"

    def ws_url(self):
        return (
            WS_URL_ENV
            or self.settings().get("line_source_ws_url")
            or WS_URL_FALLBACK
        )

    def heartbeat(self, ws_up):
        """Publish what only this process knows: that it is alive, whether
        Textractor is actually attached, and whether its writes are landing.

        `#read`'s live badge is otherwise the browser's own SSE connection,
        which stays perfectly healthy while nothing at all is being captured.
        """
        self.attached = bool(ws_up)
        if not self.pending:
            self._send([])

    def add(self, ts, text, ruby=None):
        """One captured line, on its way to the ledger.

        The character count is deliberately not computed here: it is the
        ledger's rule, and two implementations of it drift into two different
        answers for chars/h.
        """
        if self.disabled:
            return
        self.pending.append(
            {"ts": ts, "text": text, "ruby": ruby or None}
        )
        # Under an outage this keeps the newest lines; the oldest are the ones
        # already in lines.log the longest, so they stay recoverable from there.
        del self.pending[: -self.MAX_PENDING]
        self.flush()

    def retract_last(self):
        """Take the previous line back out of the feed, the merged one replacing
        it. Discarded rather than deleted — the reader's own clear button works
        the same way, and an id already handed to `term_surfaces` or crossed by
        an ingest watermark must stay resolvable."""
        if self.pending:
            self.pending.pop()
            return
        if not self.ready():
            return
        try:
            post_json("/api/lines/retract", {"source": SOURCE})
        except (OSError, ValueError) as e:
            log(f"could not retract the split line ({e}) — both halves stand")

    def flush(self):
        if not self.pending:
            return
        self._send(self.pending)

    def _send(self, lines):
        """One request carrying whatever is held, plus this process's health.

        The health rides along rather than going in its own request: every
        flush is already saying the source is alive, and a separate beat would
        double the traffic to say it again.
        """
        if not self.ready():
            return
        stuck = len(lines) > 1
        try:
            post_json(
                "/api/lines",
                {
                    "source": SOURCE,
                    "lines": lines,
                    "status": {"attached": self.attached, "pending": len(lines)},
                },
            )
        except (OSError, ValueError) as e:
            if not stuck:
                self._unreachable(e, "ledger unreachable")
            return
        self._reached()
        if stuck:
            log(f"ledger recovered, wrote {len(lines)} held lines")
        self.pending.clear()


def log(msg):
    print(f"vn-ws-logger: {msg}", file=sys.stderr, flush=True)


def normalize(msg):
    text = msg.replace("\r", " ").replace("\n", " ").strip()
    return text[:4000]


def emit(raw, out, stats, last):
    """One captured line, whatever produced it, through to both sinks.

    `last` is the line before, as (text, ruby) — the dedup needs its text and a
    continuation needs both halves to rebuild the ruby offsets. Returns the new
    `last`, unchanged when the line was dropped.

    Shared by both sources on purpose: everything here — the cleaning, the ruby
    split, the continuation join, the dedup — is what makes a captured string a
    line, and it must not depend on which hooker sent it.
    """
    last_text, last_ruby = last
    try:
        with open(RAW_LOG, "a", encoding="utf-8") as rawlog:
            rawlog.write(f"{time.time():.9f}\t{raw!r}\n")
    except OSError:
        pass
    capture = normalize(raw)
    continuation = continues_previous(capture)
    text = clean_line(capture)
    if not text:
        return last
    # The break the halves are rejoined on below, so it is not counted twice
    # and every ruby offset stays measured from the first character.
    if continuation:
        text = text.lstrip("\n")
    # The log and the dedup below both want the line as written: furigana
    # is a separate layer from here on, and only the overlay draws it.
    text, ruby = split_ruby(text)
    if not text:
        return last
    if continuation and last_text is not None:
        stats.retract_last()
        shift = u16len(last_text) + 1
        ruby = [[start + shift, length, r] for start, length, r in ruby]
        text = f"{last_text}\n{text}"
        ruby = last_ruby + ruby
    # A re-hook of the line still on screen (Textractor double-fire,
    # focus change) must not move the anchor. Only the immediately
    # preceding line is suppressed, so a genuine later repeat of the
    # same short line — separated by other dialogue — still logs.
    elif text == last_text:
        return last
    ts = time.time()
    out.write(f"{ts:.9f}\t{text}\n")
    out.flush()
    stats.add(ts, text, ruby)
    return text, ruby


async def read_lines(ws, out, stats, last):
    """Drain one WebSocket connection into the log and the stats sink."""
    async for raw in ws:
        if isinstance(raw, bytes):
            raw = raw.decode("utf-8", "replace")
        last = emit(raw, out, stats, last)
    return last


async def watch_pause(ws, stats):
    """Close the connection cleanly once capture is paused, or once the reader
    has switched to another source.

    A separate task because the read loop is parked in `async for`: this is what
    turns the flag into an actual disconnect rather than a filter.
    """
    url = stats.ws_url()
    while True:
        await asyncio.sleep(PAUSE_POLL_SECS)
        if stats.capture_paused():
            log("capture paused — closing the Textractor connection")
        elif stats.line_source() != "ws":
            log("line source changed — closing the Textractor connection")
        elif stats.ws_url() != url:
            log("WebSocket address changed — reconnecting")
        else:
            continue
        await ws.close()
        return


def clipboard_reader():
    """A command that prints the clipboard, or None when nothing can.

    wl-paste first: it is the one that works on a Wayland session, which
    includes reading an XWayland game. `-n` because a trailing newline is not
    part of what was copied and would make every line look changed.
    """
    for cmd in (
        ["wl-paste", "-n", "--no-newline"],
        ["xclip", "-o", "-selection", "clipboard"],
        ["xsel", "-b", "-o"],
    ):
        if shutil.which(cmd[0]):
            return cmd
    return None


def read_clipboard(cmd):
    """The clipboard's text, or None when it holds nothing readable.

    An empty clipboard, an image, or a selection owner that has gone away all
    make the tool fail or print nothing — none of which is a line, and none of
    which is worth a log entry every quarter second.
    """
    try:
        out = subprocess.run(cmd, capture_output=True, timeout=2)
    except (OSError, subprocess.SubprocessError):
        return None
    if out.returncode != 0:
        return None
    text = out.stdout.decode("utf-8", "replace")
    return text or None


async def pump_clipboard(out, stats, state, last):
    """Poll the clipboard for as long as it is the chosen source.

    Whatever is on the clipboard at the moment this starts is the *previous*
    value, never a line: a switch to this source would otherwise log the last
    thing the reader copied hours ago, and so would every restart.
    """
    cmd = clipboard_reader()
    if cmd is None:
        log("no clipboard tool — install wl-clipboard (Wayland) or xclip (X11)")
        await asyncio.sleep(RECONNECT_SECS)
        return last
    log(f"watching the clipboard with {cmd[0]}")
    seen = read_clipboard(cmd)
    state["ws"] = True
    try:
        while True:
            await asyncio.sleep(CLIPBOARD_POLL_SECS)
            if stats.capture_paused() or stats.line_source() != "clipboard":
                log("clipboard watch stopping")
                return last
            text = read_clipboard(cmd)
            if text is None or text == seen:
                continue
            seen = text
            last = emit(text, out, stats, last)
    finally:
        state["ws"] = None


async def pump(out, stats, state):
    last = (None, [])
    # An explicit loop rather than `async for ws in websockets.connect(...)`:
    # that form reconnects on its own, which is exactly what a pause must not
    # do. Reconnecting is now a decision made here, once per iteration, after
    # the flag and the chosen source have been checked.
    while True:
        if stats.capture_paused():
            await asyncio.sleep(PAUSE_POLL_SECS)
            continue
        if stats.line_source() == "clipboard":
            last = await pump_clipboard(out, stats, state, last)
            continue
        url = stats.ws_url()
        try:
            async with websockets.connect(
                url, max_size=None, ping_interval=20, ping_timeout=20
            ) as ws:
                log(f"connected to {url}")
                state["ws"] = ws
                watcher = asyncio.create_task(watch_pause(ws, stats))
                try:
                    last = await read_lines(ws, out, stats, last)
                except websockets.ConnectionClosed:
                    log("connection closed")
                finally:
                    watcher.cancel()
                    state["ws"] = None
        except (OSError, websockets.WebSocketException) as e:
            log(f"connect to {url} failed ({e}), retrying")
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
