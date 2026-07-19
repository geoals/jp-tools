#!/usr/bin/env python3
"""Trim a VN voiceline clip to the mined sentence.

The ring-buffer clip starts at the hooked line's timestamp, so when Yomitan
mines only one sentence out of a multi-sentence line the clip carries extra
speech. This locates the mined sentence inside the clip via whisper word
timestamps and prints the window to keep.

Usage: vn-trim.py <wav-16k-mono> <target_word> <sentence> [whisper_url]

stdout: "START END" (seconds to keep), or "none" when no trim is needed or
no confident match was found. Callers should keep the full clip on "none"
or a nonzero exit — failure never makes the clip worse.

Matching strategy:
1. difflib-align the mined sentence against the punctuation-stripped
   transcript (robust to wrong-kanji ASR: same-reading substitutions leave
   most chars aligned).
2. If alignment is weak, anchor on the target word and expand to the
   nearest sentence-ending punctuation or inter-word silence gap.
"""

import difflib
import json
import os
import re
import subprocess
import sys
import wave

PRE_PAD = 0.30
POST_PAD = 0.25
SNAP_TOL = 0.25      # how far a VAD silence may sit from a whisper word edge
VAD_SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "vn-vad.py")
MIN_COVERAGE = 0.6   # fraction of sentence chars that must align
MIN_BLOCK = 2        # ignore 1-char difflib blocks (spurious in Japanese)
GAP_BOUNDARY = 0.5   # inter-word silence treated as a sentence boundary (s)
END_PUNCT = "。！？!?…‥"

STRIP_RE = re.compile(r"[<>\s。、！？!?…‥「」『』（）()・〜～―─　]|<[^>]*>")


def norm_chars(text):
    """Strip HTML tags and punctuation; return list of content chars."""
    text = re.sub(r"<[^>]*>", "", text)
    return [c for c in text if not STRIP_RE.match(c)]


def transcribe(wav_path, url, words=True):
    query = "?words=true" if words else ""
    out = subprocess.run(
        ["curl", "-s", "-m", "30", "-X", "POST", f"{url}/transcribe{query}",
         "-F", f"audio=@{wav_path}"],
        capture_output=True, text=True, check=True,
    ).stdout
    result = []
    text = ""
    for line in out.splitlines():
        seg = json.loads(line)
        result.extend(seg.get("words") or [])
        text += seg.get("text", "")
    return result if words else text


def cut_wav(src, dst, start, end):
    with wave.open(src) as w:
        sr = w.getframerate()
        w.setpos(int(start * sr))
        frames = w.readframes(int((end - start) * sr))
    with wave.open(dst, "wb") as o:
        o.setnchannels(1)
        o.setsampwidth(2)
        o.setframerate(sr)
        o.writeframes(frames)


def align_sentence(norm_sent, tx_chars, char_word, words):
    """difflib-align the sentence; return (first_word, last_word, coverage)."""
    norm_tx = "".join(tx_chars)
    sm = difflib.SequenceMatcher(None, "".join(norm_sent), norm_tx, autojunk=False)
    blocks = [b for b in sm.get_matching_blocks() if b.size >= MIN_BLOCK]
    if not blocks:
        return None
    matched = sum(b.size for b in blocks)
    first = char_word[blocks[0].b]
    last = char_word[blocks[-1].b + blocks[-1].size - 1]

    # ASR often garbles the sentence's first/last word (wrong kanji, proper
    # nouns), leaving it outside the matching blocks. Extend the span to
    # cover the unmatched sentence prefix/suffix, stopping at sentence
    # punctuation or a silence gap so we can't leak into a neighbor sentence.
    def boundary(a, b):
        return (any(p in words[a]["word"] for p in END_PUNCT)
                or words[b]["start"] - words[a]["end"] >= GAP_BOUNDARY)

    need = blocks[0].a  # sentence chars before the first matched block
    while need > 0 and first > 0 and not boundary(first - 1, first):
        first -= 1
        need -= len(norm_chars(words[first]["word"]))
    need = len(norm_sent) - (blocks[-1].a + blocks[-1].size)
    while need > 0 and last < len(words) - 1 and not boundary(last, last + 1):
        last += 1
        need -= len(norm_chars(words[last]["word"]))

    return first, last, matched / max(1, len(norm_sent))


def anchor_on_word(norm_word, words, tx_chars, char_word):
    """Fallback: find target word, expand to punctuation/silence boundaries."""
    idx = "".join(tx_chars).find("".join(norm_word))
    if idx < 0:
        return None
    first = last = char_word[idx]
    while first > 0:
        prev = words[first - 1]
        if any(p in prev["word"] for p in END_PUNCT):
            break
        if words[first]["start"] - prev["end"] >= GAP_BOUNDARY:
            break
        first -= 1
    while last < len(words) - 1:
        if any(p in words[last]["word"] for p in END_PUNCT):
            break
        if words[last + 1]["start"] - words[last]["end"] >= GAP_BOUNDARY:
            break
        last += 1
    return first, last


def silences(wav_path, duration):
    """Silence gaps between silero speech segments, or None if VAD fails."""
    r = subprocess.run([sys.executable, VAD_SCRIPT, "--segments", wav_path],
                       capture_output=True, text=True, timeout=60)
    segs = []
    for line in r.stdout.splitlines():
        parts = line.split()
        if len(parts) == 2:
            segs.append((float(parts[0]), float(parts[1])))
    if r.returncode != 0 or not segs:
        return None
    gaps = [(0.0, segs[0][0])]
    gaps += [(a[1], b[0]) for a, b in zip(segs, segs[1:])]
    gaps.append((segs[-1][1], duration))
    return [(s, e) for s, e in gaps if e > s]


def main():
    wav_path, target_word, sentence = sys.argv[1], sys.argv[2], sys.argv[3]
    url = sys.argv[4] if len(sys.argv) > 4 else "http://localhost:8100"

    with wave.open(wav_path) as w:
        duration = w.getnframes() / w.getframerate()

    words = transcribe(wav_path, url)
    if not words:
        print("none")
        return

    # Per-content-char index into words, parallel to the normalized transcript
    tx_chars, char_word = [], []
    for i, w in enumerate(words):
        for c in norm_chars(w["word"]):
            tx_chars.append(c)
            char_word.append(i)
    if not tx_chars:
        print("none")
        return

    span = None
    aligned = align_sentence(norm_chars(sentence), tx_chars, char_word, words)
    if aligned and aligned[2] >= MIN_COVERAGE:
        span = aligned[:2]
    else:
        span = anchor_on_word(norm_chars(target_word), words, tx_chars, char_word)
    if span is None:
        print("none")
        return

    # Whisper word-edge timestamps are sloppy (ends especially run early in
    # Japanese), so prefer snapping each cut into the silero silence gap
    # nearest the matched span's edge; whisper timing is only a fallback.
    first, last = span
    w_start, w_end = words[first]["start"], words[last]["end"]
    gaps = silences(wav_path, duration) or []

    start = None
    for gs, ge in reversed(gaps):
        if ge <= w_start + SNAP_TOL and (first == 0 or ge > words[first - 1]["end"] - SNAP_TOL):
            start = max(ge - PRE_PAD, gs)
            break
    if start is None:
        start = w_start - PRE_PAD
        if first > 0:
            start = max(start, (words[first - 1]["end"] + w_start) / 2)
    start = max(0.0, start)

    next_start = words[last + 1]["start"] if last < len(words) - 1 else duration
    end = None
    for gs, ge in gaps:
        if gs >= w_end - SNAP_TOL and gs <= next_start + SNAP_TOL:
            end = min(gs + POST_PAD, ge)
            break
    if end is None:
        # no silence before the next sentence — cut just ahead of its onset
        end = max(w_end, min(w_end + POST_PAD, next_start - 0.05))
    end = min(duration, end)

    # Verify the tail survived: voice actors stretch final syllables (よ……)
    # past whisper's word end, and with no silence gap both timing sources
    # lie. Re-transcribe the cut; while the sentence's final char is missing,
    # push the end out. (The head needs no check — garbled ASR there is
    # already covered by the prefix extension in align_sentence.)
    norm_sent = norm_chars(sentence)
    end_cap = min(duration, max(end, next_start) + 1.0)
    check_wav = wav_path + ".check.wav"
    try:
        for _ in range(4):
            cut_wav(wav_path, check_wav, start, end)
            heard = norm_chars(transcribe(check_wav, url, words=False))
            sm = difflib.SequenceMatcher(None, "".join(norm_sent), "".join(heard), autojunk=False)
            blocks = [b for b in sm.get_matching_blocks() if b.size >= MIN_BLOCK]
            if blocks and blocks[-1].a + blocks[-1].size >= len(norm_sent):
                break
            if end >= end_cap:
                break
            end = min(end + 0.3, end_cap)
    finally:
        if os.path.exists(check_wav):
            os.unlink(check_wav)

    # Already covers (nearly) the whole clip — not worth re-cutting
    if start < 0.35 and end > duration - 0.3:
        print("none")
        return
    print(f"{start:.3f} {end:.3f}")


if __name__ == "__main__":
    try:
        main()
    except Exception as e:
        print(f"vn-trim: {e}", file=sys.stderr)
        sys.exit(1)
