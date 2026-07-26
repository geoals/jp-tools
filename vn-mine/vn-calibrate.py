#!/usr/bin/env python
"""Measure the two things a smarter vn-capture.sh would like to assume.

  1. **Onset.** If a line is voiced, its voice should start at the moment
     Textractor hooked it. How closely, and with how much spread, decides
     whether "is there speech starting right here" can replace "is there speech
     anywhere in the next ten seconds" — a check at a known point instead of a
     search, and the only test that tells an unvoiced line from a voiced one
     without needing to hear the difference.
  2. **Duration.** How well the line's own length predicts how long its
     voiceline runs, in morae per second. A clip far outside that is either two
     lines merged or something that is not this line at all.

Run it *during or just after reading* — it reads the live ring buffer, which
holds only the last ~5 minutes, and it needs dialogue in that window. Idle on a
menu with BGM gives nothing: the ring will be loud and VAD will find no speech.

    vn-mine/vn-calibrate.py [workdir]

Read-only against the ring and the log; writes only into `workdir` (default a
temp dir). Safe to run mid-session.
"""

import os
import re
import subprocess
import sys
import tempfile
import wave
from pathlib import Path

RUNDIR = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}")) / "vn-mine"
SEGDIR = RUNDIR / "seg"
LINES = RUNDIR / "lines.log"
BPS = 192000  # 48 kHz * 2 ch * 2 bytes
HDR = 44
VAD_PYTHON = Path.home() / ".local/share/vn-mine/venv/bin/python"
VAD_SCRIPT = Path(__file__).resolve().parent / "vn-vad.py"

# One voiceline breathes; segments closer than this are the same utterance.
# Deliberately tighter than vn-capture.sh's 1.0s merge, which is wide enough to
# swallow the following line — the thing being measured here.
MERGE = 0.35

# How far from the hook a candidate utterance may start to be considered at
# all. Wide on purpose: the point is to see the distribution, not enforce a rule.
SEARCH = (-1.5, 4.0)

# Inside this, an utterance counts as "started at the hook" for the rate
# statistics. Also deliberately loose — tightening it is what the numbers are for.
VOICED = 1.0

KANA = re.compile(r"[ぁ-ゟァ-ヿ]")
KANJI = re.compile(r"[一-鿿]")
SMALL = set("ゃゅょャュョぁぃぅぇぉァィゥェォ")


def morae(text):
    """Rough mora count: kana one each (small kana ride the preceding mora),
    kanji about two. Punctuation, latin and 【speaker】 tags do not sound.

    Deliberately crude — the question is whether even this predicts duration
    well enough to bound a search window, not whether it is a good reading of
    the text. A real answer would need the reading, which is not on hand here.
    """
    text = re.sub(r"【[^】]*】", "", text)
    n = 0
    for c in text:
        if c in SMALL:
            continue
        if KANA.match(c):
            n += 1
        elif KANJI.match(c):
            n += 2
    return n


def build_stream(work):
    """The ring as one contiguous 16 kHz mono WAV, plus the epoch time its
    first sample was recorded at."""
    segs = sorted(SEGDIR.glob("seg*.wav"), key=lambda p: p.stat().st_mtime)
    if not segs:
        sys.exit(f"ring is empty ({SEGDIR}) — is vn-buffer.service running?")
    total = sum(p.stat().st_size - HDR for p in segs)
    stream_end = segs[-1].stat().st_mtime
    stream_start = stream_end - total / BPS

    raw = work / "stream.raw"
    with open(raw, "wb") as out:
        for p in segs:
            with open(p, "rb") as f:
                f.seek(HDR)
                out.write(f.read())

    wav = work / "stream.wav"
    subprocess.run(
        ["ffmpeg", "-nostdin", "-loglevel", "error", "-f", "s16le", "-ar",
         "48000", "-ac", "2", "-i", str(raw), "-ac", "1", "-ar", "16000",
         "-c:a", "pcm_s16le", str(wav), "-y"],
        check=True,
    )
    print(f"ring: {total / BPS:.0f}s of audio ending {stream_end:.0f}")
    return wav, stream_start


def utterances(wav, stream_start):
    out = subprocess.run(
        [str(VAD_PYTHON), str(VAD_SCRIPT), str(wav), "--segments"],
        capture_output=True, text=True,
    )
    segs = []
    for line in out.stdout.split("\n"):
        line = line.strip()
        if not line or line == "none":
            continue
        s, e = line.split()
        segs.append([stream_start + float(s), stream_start + float(e)])

    merged = []
    for s, e in segs:
        if merged and s - merged[-1][1] < MERGE:
            merged[-1][1] = e
        else:
            merged.append([s, e])
    print(f"VAD: {len(segs)} segments -> {len(merged)} utterances "
          f"after a {MERGE}s merge")
    return merged


def main():
    work = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(tempfile.mkdtemp())
    work.mkdir(parents=True, exist_ok=True)
    wav, stream_start = build_stream(work)
    with wave.open(str(wav)) as w:
        stream_end = stream_start + w.getnframes() / w.getframerate()
    merged = utterances(wav, stream_start)

    rows = []
    for raw_line in LINES.read_text(errors="replace").splitlines():
        ts_s, _, text = raw_line.partition("\t")
        try:
            ts = float(ts_s)
        except ValueError:
            continue
        # Room either side, so a line at the very edge isn't scored against
        # audio the ring no longer holds.
        if not (stream_start + 1 < ts < stream_end - 4):
            continue
        cand = [u for u in merged if SEARCH[0] <= u[0] - ts <= SEARCH[1]]
        rows.append((ts, text, morae(text), cand[0] if cand else None))

    if not rows:
        sys.exit("no hooked lines inside the ring's window — read for a few "
                 "minutes and run this again")

    print(f"\n{'onset':>7} {'dur':>6} {'morae':>6} {'mora/s':>7}  line")
    print("-" * 78)
    onsets, rates, silent = [], [], 0
    for ts, text, m, cand in rows:
        onset = dur = None
        if cand:
            onset, dur = cand[0] - ts, cand[1] - cand[0]
        rate = ""
        if onset is not None and abs(onset) < VOICED and dur and m:
            onsets.append(onset)
            rates.append(m / dur)
            rate = f"{m / dur:.1f}"
        else:
            silent += 1
        shown_onset = f"{onset:+.2f}" if onset is not None else "—"
        shown_dur = f"{dur:.2f}" if dur is not None else "—"
        print(f"{shown_onset:>7} {shown_dur:>6} {m:>6} {rate:>7}  {text[:44]}")

    print(f"\nlines in window: {len(rows)}   voiced-looking: {len(onsets)}   "
          f"no utterance at the hook: {silent}")
    if not onsets:
        print("nothing started at a hook — was there dialogue in this window?")
        return
    onsets.sort()
    rates.sort()

    def pct(xs, p):
        return xs[min(len(xs) - 1, int(len(xs) * p))]

    print(f"onset  median {pct(onsets, .5):+.2f}s   p10 {pct(onsets, .1):+.2f}"
          f"   p90 {pct(onsets, .9):+.2f}   min {onsets[0]:+.2f}"
          f"   max {onsets[-1]:+.2f}")
    print(f"rate   median {pct(rates, .5):.1f} morae/s   "
          f"p10 {pct(rates, .1):.1f}   p90 {pct(rates, .9):.1f}   "
          f"min {rates[0]:.1f}   max {rates[-1]:.1f}")
    print("\nonset spread sets the gate width; rate spread sets how loose a "
          "duration bound has to be to never cut a real voiceline.")


if __name__ == "__main__":
    main()
