#!/usr/bin/env python
# Silero-VAD (v5 onnx) speech boundary detection for vn-capture.sh
# Usage: vn-vad.py file.wav   (16 kHz mono s16 WAV)
# Prints "FIRST_START LAST_END" in seconds, or "none" if no speech found.
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
    with wave.open(sys.argv[1], "rb") as w:
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
        if merged and s * frame - merged[-1][1] * frame < MERGE_GAP:
            merged[-1] = (merged[-1][0], e)
        else:
            merged.append((s, e))
    merged = [(s, e) for s, e in merged if (e - s) * frame >= MIN_SPEECH]

    if not merged:
        print("none")
    else:
        print(f"{merged[0][0] * frame:.3f} {merged[-1][1] * frame:.3f}")


if __name__ == "__main__":
    main()
