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

The ring holds only the last ~5 minutes, so a single pass sees a few dozen
lines — enough to show that onset clusters at zero, not enough to set a
threshold. Hence `--collect`, which samples the ring on a timer and keeps the
union across a whole session.

    vn-mine/vn-calibrate.py                     one pass, printed
    vn-mine/vn-calibrate.py --collect [FILE] [INTERVAL]
    vn-mine/vn-calibrate.py --summarize FILE

Run any of them *while reading*: idle on a menu with BGM gives nothing (the
ring will be loud and VAD will find no speech). Read-only against the ring and
the log; `--collect` writes one TSV, default
`~/.local/share/jp-tools/vn-onset-calibration.tsv`, and is resumable — stop and
restart it across sessions and it keeps accumulating.

See vn-mine/README.md for what the numbers are for.
"""

import os
import re
import subprocess
import sys
import tempfile
import time
import wave
from pathlib import Path

DEFAULT_LOG = Path.home() / ".local/share/jp-tools/vn-onset-calibration.tsv"

RUNDIR = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}")) / "vn-mine"
SEGDIR = RUNDIR / "seg"
LINES = RUNDIR / "lines.log"
BPS = 192000  # 48 kHz * 2 ch * 2 bytes
HDR = 44
VAD_PYTHON = Path.home() / ".local/share/vn-mine/venv/bin/python"
VAD_SCRIPT = Path(__file__).resolve().parent / "vn-vad.py"

# One voiceline breathes; segments closer than this are the same utterance.
#
# Measured, not guessed: at 0.35s a delivered line splits on its own internal
# pauses and the median speaking rate comes out at 18 morae/s, which is roughly
# twice what Japanese speech runs at. At 1.0s the median is 8.4 — where real
# speech sits — so anything tighter is measuring fragments, not voicelines.
MERGE = float(os.environ.get("VN_CAL_MERGE", "1.0"))

# How far from the hook a candidate utterance may start to be considered at
# all. Wide on purpose: the point is to see the distribution, not enforce a rule.
SEARCH = (-1.5, 4.0)

# NOTE: no "is it voiced" threshold is applied before the statistics. Selecting
# the sample by |onset| < x and then reporting that onsets fall within x is
# circular, and that is exactly what the first version of this did. Every
# matched line is reported; the shape of the raw column is the evidence.

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


def score(work):
    """One pass over the ring: every line inside it, with the utterance that
    belongs to it (or None). Returns `[[ts, text, morae, (start, end)|None], …]`
    sorted by time, plus the count of utterances no line claimed."""
    wav, stream_start = build_stream(work)
    with wave.open(str(wav)) as w:
        stream_end = stream_start + w.getnframes() / w.getframerate()
    merged = utterances(wav, stream_start)

    lines = []
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
        lines.append([ts, text, morae(text), None])

    if not lines:
        sys.exit("no hooked lines inside the ring's window — read for a few "
                 "minutes and run this again")
    lines.sort(key=lambda r: r[0])

    # Assign each utterance to exactly ONE line: the last line hooked at or
    # just before it starts. Letting every line search the window independently
    # is what the capture bug *is* — an unvoiced narration line happily claims
    # the following line's voice — and a measuring tool that reproduces it
    # reports every silent line as voiced and invents impossible speaking
    # rates. One utterance, one owner, nearest preceding line.
    unclaimed = 0
    for u_start, u_end in merged:
        owner = None
        for row in lines:
            if row[0] <= u_start - SEARCH[0]:
                owner = row
            else:
                break
        # An utterance may legitimately start a little before its line is
        # hooked, but not after the *next* line has been.
        if owner is None or (owner[3] is not None) or u_start - owner[0] > SEARCH[1]:
            unclaimed += 1
            continue
        owner[3] = (u_start, u_end)
    return lines, unclaimed


def report(lines, unclaimed):
    print(f"\n{'onset':>7} {'dur':>6} {'morae':>6} {'mora/s':>7}  line")
    print("-" * 78)
    onsets, rates, silent = [], [], 0
    for ts, text, m, cand in lines:
        onset = dur = None
        if cand:
            onset, dur = cand[0] - ts, cand[1] - cand[0]
        rate = ""
        if onset is not None and dur and m:
            onsets.append(onset)
            rates.append(m / dur)
            rate = f"{m / dur:.1f}"
        else:
            silent += 1
        shown_onset = f"{onset:+.2f}" if onset is not None else "—"
        shown_dur = f"{dur:.2f}" if dur is not None else "—"
        print(f"{shown_onset:>7} {shown_dur:>6} {m:>6} {rate:>7}  {text[:44]}")

    print(f"\nlines in window: {len(lines)}   matched an utterance: "
          f"{len(onsets)}   silent at the hook: {silent}   "
          f"utterances claimed by no line: {unclaimed}")
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


# --- accumulating collection -------------------------------------------------
#
# One ring holds five minutes, which was eleven matched lines — enough to see
# that onset clusters at zero, nowhere near enough to decide whether the +2s
# group is a real mode or three bad matches. Sampling the ring on a timer and
# keeping the union across a whole session is the difference between a hunch
# and a threshold.

COLUMNS = "line_ts\tonset\tdur\tmorae\tchars\ttext"


def collect(path, work, interval):
    """Sample the ring every `interval` seconds, appending lines not seen
    before. Overlapping windows are the point — a line scored near the edge of
    one ring is scored again, mid-ring, by the next pass; `keep_better` prefers
    the reading with more audio around it."""
    seen = {}
    if path.exists():
        for row in path.read_text().splitlines()[1:]:
            parts = row.split("\t")
            if len(parts) == 6:
                seen[parts[0]] = parts
        print(f"resuming: {len(seen)} lines already recorded in {path}")
    else:
        path.write_text(COLUMNS + "\n")

    while True:
        try:
            lines, _ = score(work)
        except SystemExit as e:  # empty ring, no lines yet — keep waiting
            print(f"skip: {e}", flush=True)
            lines = []
        added = 0
        for ts, text, m, cand in lines:
            key = f"{ts:.6f}"
            onset = f"{cand[0] - ts:.3f}" if cand else ""
            dur = f"{cand[1] - cand[0]:.3f}" if cand else ""
            row = [key, onset, dur, str(m), str(len(text)), text]
            if key not in seen:
                added += 1
            elif not keep_better(row, seen[key]):
                continue
            seen[key] = row
        with open(path, "w") as f:
            f.write(COLUMNS + "\n")
            for key in sorted(seen, key=float):
                f.write("\t".join(seen[key]) + "\n")
        matched = sum(1 for r in seen.values() if r[1])
        print(f"{time.strftime('%H:%M:%S')}  +{added} new   "
              f"{len(seen)} lines   {matched} with audio   -> {path}",
              flush=True)
        time.sleep(interval)


def keep_better(new, old):
    """A line seen twice: prefer the pass that found audio for it. A line at the
    very start of a ring can have its voice cut off by the ring's own edge, and
    that reads as silence — which is precisely the thing being counted, so it
    must not be recorded from a window that could not have seen it."""
    return bool(new[1]) and not old[1]


def summarize(path):
    rows = [r.split("\t") for r in path.read_text().splitlines()[1:]]
    rows = [r for r in rows if len(r) == 6]
    onsets = sorted(float(r[1]) for r in rows if r[1])
    rates = sorted(
        int(r[3]) / float(r[2]) for r in rows if r[2] and float(r[2]) > 0 and int(r[3])
    )
    print(f"{len(rows)} lines, {len(onsets)} with an utterance at the hook")
    if not onsets:
        return

    def pct(xs, p):
        return xs[min(len(xs) - 1, int(len(xs) * p))]

    for name, xs, unit in (("onset", onsets, "s"), ("rate", rates, " morae/s")):
        if not xs:
            continue
        print(f"{name:6} min {xs[0]:+.2f}  p05 {pct(xs, .05):+.2f}  "
              f"p25 {pct(xs, .25):+.2f}  median {pct(xs, .5):+.2f}  "
              f"p75 {pct(xs, .75):+.2f}  p95 {pct(xs, .95):+.2f}  "
              f"max {xs[-1]:+.2f}{unit}")
    # The shape that matters is whether the onsets are one cluster or two.
    print("\nonset histogram (0.25s bins):")
    lo = int(min(onsets) * 4) - 1
    hi = int(max(onsets) * 4) + 1
    for b in range(lo, hi + 1):
        n = sum(1 for o in onsets if b <= o * 4 < b + 1)
        if n:
            print(f"  {b / 4:+.2f}..{(b + 1) / 4:+.2f}  {'#' * n} {n}")


def main():
    args = sys.argv[1:]
    work = Path(tempfile.mkdtemp())

    if args and args[0] == "--summarize":
        return summarize(Path(args[1]))

    if args and args[0] == "--collect":
        path = Path(args[1]) if len(args) > 1 else DEFAULT_LOG
        interval = float(args[2]) if len(args) > 2 else 210.0
        path.parent.mkdir(parents=True, exist_ok=True)
        print(f"collecting into {path} every {interval:.0f}s — Ctrl-C to stop")
        return collect(path, work, interval)

    if args:
        work = Path(args[0])
        work.mkdir(parents=True, exist_ok=True)
    report(*score(work))


if __name__ == "__main__":
    main()
