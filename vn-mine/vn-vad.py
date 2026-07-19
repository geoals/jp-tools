#!/usr/bin/env python
# Silero-VAD (v5 onnx) speech boundary detection for vn-capture.sh
# Usage: vn-vad.py file.wav   (16 kHz mono s16 WAV)
# Prints "FIRST_START LAST_END" in seconds, or "none" if no speech found.
# With --segments: one "START END" line per speech segment, finer merging —
# used by vn-trim.py to snap sentence cuts to real silence boundaries.
# Env: VN_VAD_THRESHOLD (default 0.5), VN_VAD_MODEL (path to onnx model)

import os
import sys
import wave

import numpy as np
import onnxruntime as ort

SR = 16000
CHUNK = 512          # samples per VAD frame at 16 kHz
CONTEXT = 64         # v5 model expects 64 context samples prepended
THRESHOLD = float(os.environ.get("VN_VAD_THRESHOLD", "0.5"))
MODEL = os.environ.get(
    "VN_VAD_MODEL",
    os.path.expanduser("~/.local/share/vn-mine/silero_vad.onnx"),
)
MIN_SPEECH = 0.25    # ignore blips shorter than this
MERGE_GAP = 1.0      # merge speech segments separated by less than this


def main():
    seg_mode = "--segments" in sys.argv[1:]
    wav_path = next(a for a in sys.argv[1:] if a != "--segments")
    merge_gap = 0.15 if seg_mode else MERGE_GAP
    min_speech = 0.1 if seg_mode else MIN_SPEECH
    with wave.open(wav_path, "rb") as w:
        assert w.getframerate() == SR and w.getnchannels() == 1 and w.getsampwidth() == 2
        pcm = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16)
    audio = pcm.astype(np.float32) / 32768.0

    sess = ort.InferenceSession(MODEL, providers=["CPUExecutionProvider"])
    state = np.zeros((2, 1, 128), dtype=np.float32)
    context = np.zeros(CONTEXT, dtype=np.float32)
    sr = np.array(SR, dtype=np.int64)

    probs = []
    for off in range(0, len(audio) - CHUNK + 1, CHUNK):
        chunk = audio[off : off + CHUNK]
        x = np.concatenate([context, chunk])[np.newaxis, :]
        out, state = sess.run(None, {"input": x, "state": state, "sr": sr})
        probs.append(float(out[0, 0]))
        context = chunk[-CONTEXT:]

    frame = CHUNK / SR
    segments = []  # [start_frame, end_frame) of consecutive speech
    start = None
    for i, p in enumerate(probs):
        if p >= THRESHOLD and start is None:
            start = i
        elif p < THRESHOLD and start is not None:
            segments.append((start, i))
            start = None
    if start is not None:
        segments.append((start, len(probs)))

    merged = []
    for s, e in segments:
        if merged and s * frame - merged[-1][1] * frame < merge_gap:
            merged[-1] = (merged[-1][0], e)
        else:
            merged.append((s, e))
    merged = [(s, e) for s, e in merged if (e - s) * frame >= min_speech]

    if not merged:
        print("none")
    elif seg_mode:
        for s, e in merged:
            print(f"{s * frame:.3f} {e * frame:.3f}")
    else:
        print(f"{merged[0][0] * frame:.3f} {merged[-1][1] * frame:.3f}")


if __name__ == "__main__":
    main()
