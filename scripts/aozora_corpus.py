#!/usr/bin/env python3
"""Fetch a corpus of public-domain Japanese prose from Aozora Bunko.

Aozora Bunko publishes works whose copyright has expired, which is what makes
them usable here. The authors below all died well over seventy years ago.

The corpus is what the demo's line stream is written from. It has to be real
prose rather than a sentence bank: the kanji grid and "new kanji per day" are
counting distinct characters, and forty repeated sentences give 154 kanji all
met the same number of times on one day, which is exactly what a real reading
history does not look like.

Cached, because it is a few MB over a few dozen HTTP requests.
"""

import re
import sys
import time
import urllib.request
from pathlib import Path

# Author index pages. Order is the order the corpus is built in, and the demo
# walks it front to back, so this is also the order kanji are first met.
AUTHORS = [
    ("000148", "夏目漱石"),
    ("000879", "芥川龍之介"),
    ("000129", "森鴎外"),
    ("000035", "太宰治"),
    ("000081", "宮沢賢治"),
    ("000119", "中島敦"),
    ("000051", "有島武郎"),
    ("000038", "国木田独歩"),
]

UA = {"User-Agent": "kotodex-demo-seed/1.0 (+https://kotodex.com)"}

# The character allowlist `jp_core::text::chars::is_counted` uses. The corpus is
# sized in these rather than in raw characters, because that is the unit the
# reading history is measured in and the unit the replacement has to match.
COUNTED_RANGES = [
    (0x30, 0x39), (0x41, 0x5A), (0x61, 0x7A),
    (0x25CB, 0x25CB), (0x25EF, 0x25EF),
    (0x3005, 0x3007), (0x303B, 0x303B),
    (0x3041, 0x3096), (0x309D, 0x309E),
    (0x30A1, 0x30FA), (0x30FC, 0x30FC),
    (0xFF10, 0xFF19), (0xFF21, 0xFF3A), (0xFF41, 0xFF5A), (0xFF66, 0xFF9D),
    (0x2E80, 0x2E99), (0x2E9B, 0x2EF3), (0x2F00, 0x2FD5),
    (0x3400, 0x4DBF), (0x4E00, 0x9FFF),
    (0x20000, 0x2A6DF), (0x2A700, 0x2B81D), (0x2B820, 0x2CEAD),
]


def _counted_set() -> frozenset:
    out = set()
    for lo, hi in COUNTED_RANGES:
        # The astral ranges are millions of codepoints and essentially never
        # appear, so they stay on a range check rather than going in the set.
        if hi <= 0xFFFF:
            out.update(range(lo, hi + 1))
    return frozenset(out)


COUNTED = _counted_set()
ASTRAL = [(lo, hi) for lo, hi in COUNTED_RANGES if hi > 0xFFFF]


def is_counted(ch: str) -> bool:
    o = ord(ch)
    return o in COUNTED or any(lo <= o <= hi for lo, hi in ASTRAL)


def count_chars(text: str) -> int:
    counted = COUNTED
    return sum(1 for ch in text if ord(ch) in counted)


def get(url: str) -> str:
    request = urllib.request.Request(url, headers=UA)
    raw = urllib.request.urlopen(request, timeout=40).read()
    # Card and text pages are Shift_JIS; the index pages are UTF-8. The meta tag
    # is the only reliable signal, and it is ASCII either way.
    head = raw[:2048].decode("ascii", errors="replace").lower()
    encoding = "utf-8" if "utf-8" in head else "shift_jis"
    return raw.decode(encoding, errors="replace")


def clean(html: str) -> str:
    """Aozora's HTML down to the body text.

    Ruby is dropped rather than kept: the reading is an annotation, and leaving
    it in would put every furigana kana into the character count and the kanji
    grid's denominators.
    """
    body = re.search(
        r'<div class="main_text">(.*?)</div>', html, re.S | re.I
    )
    if not body:
        return ""
    text = body.group(1)
    text = re.sub(r"<rp>.*?</rp>", "", text, flags=re.S)
    text = re.sub(r"<rt>.*?</rt>", "", text, flags=re.S)
    text = re.sub(r"<[^>]+>", "", text)
    # Editorial notes: ［＃「…」は底本では…］ and the like.
    text = re.sub(r"［＃.*?］", "", text, flags=re.S)
    text = text.replace("&nbsp;", "").replace("&amp;", "&")
    text = re.sub(r"[ \t　]+", "", text)
    text = re.sub(r"\n{2,}", "\n", text)
    return text.strip()


def works_for(person: str, limit: int):
    # The index page drops the zero padding the card paths keep.
    index = get(
        f"https://www.aozora.gr.jp/index_pages/person{int(person)}.html"
    )
    cards = []
    for p, c in re.findall(r"cards/(\d+)/card(\d+)\.html", index):
        if (p, c) not in cards:
            cards.append((p, c))
    return cards[:limit]


def build(target_chars: int, out: Path) -> str:
    """Fetch until the corpus covers `target_chars` counted characters.

    It stops the moment it has enough, mid-work if that is where the total lands
    — the corpus is sized to one reader's history, not assembled as a library.
    """
    seen_path = out.with_suffix(".cards")
    seen: set[str] = set()
    pieces, total = [], 0
    # Resume from whatever is already on disk. Without this, asking for a little
    # more text re-downloads everything already fetched.
    if out.exists() and seen_path.exists():
        cached = out.read_text(encoding="utf-8")
        if cached:
            pieces.append(cached)
            total = count_chars(cached)
            seen = set(seen_path.read_text().split())
            print(f"  resuming: {total} counted from {len(seen)} works")

    for person, name in AUTHORS:
        if total >= target_chars:
            break
        try:
            cards = works_for(person, 40)
        except Exception as e:
            print(f"  {name}: index unavailable ({e})")
            continue
        for card_person, card in cards:
            if total >= target_chars:
                break
            if card in seen:
                continue
            try:
                page = get(
                    f"https://www.aozora.gr.jp/cards/{card_person}/card{card}.html"
                )
                files = re.findall(r'href="[./]*?(files/\d+_\d+\.html)"', page)
                if not files:
                    continue
                text = clean(
                    get(f"https://www.aozora.gr.jp/cards/{card_person}/{files[0]}")
                )
            except Exception:
                continue
            if count_chars(text) < 2000:
                continue
            counted = count_chars(text)
            if total + counted > target_chars:
                # Take only the run still needed and stop.
                keep, got = [], 0
                for ch in text:
                    keep.append(ch)
                    if is_counted(ch):
                        got += 1
                    if total + got >= target_chars:
                        break
                text = "".join(keep)
                counted = got
            pieces.append(text)
            seen.add(card)
            total += counted
            print(f"  {name} card{card}: {counted:>7} counted (total {total})")
            if total >= target_chars:
                break
            time.sleep(0.3)

    corpus = "\n".join(pieces)
    out.write_text(corpus, encoding="utf-8")
    seen_path.write_text("\n".join(sorted(seen)))
    return corpus


# The corpus is read in a loop and wraps when it runs out, so falling a little
# short means the tail repeats rather than the seed failing. Not worth another
# few hundred HTTP requests to close.
ENOUGH = 0.9


def load(target_chars: int, cache: Path) -> str:
    if cache.exists():
        cached = cache.read_text(encoding="utf-8")
        have = count_chars(cached)
        if have >= target_chars * ENOUGH:
            if have < target_chars:
                print(f"  cache has {have} of {target_chars}; the tail wraps")
            return cached
    cache.parent.mkdir(parents=True, exist_ok=True)
    print(f"fetching ~{target_chars} chars from Aozora Bunko")
    return build(target_chars, cache)


if __name__ == "__main__":
    target = int(sys.argv[1]) if len(sys.argv) > 1 else 2_100_000
    path = Path(sys.argv[2]) if len(sys.argv) > 2 else Path("target/demo-corpus.txt")
    corpus = load(target, path)
    print(f"corpus: {count_chars(corpus)} counted chars -> {path}")
