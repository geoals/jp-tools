#!/usr/bin/env python3
"""Transcribe an audio file using faster-whisper.

Usage: python3 transcribe.py <audio_path> [cpu_threads] [device]

Outputs JSON to stdout: [{"start": 0.0, "end": 3.2, "text": "..."}, ...]
All progress/status messages go to stderr so they appear in the server logs.
"""

import json
import os
import sys


def main():
    if len(sys.argv) < 2:
        print("Usage: transcribe.py <audio_path> [cpu_threads]", file=sys.stderr)
        sys.exit(1)

    audio_path = sys.argv[1]
    cpu_threads = int(sys.argv[2]) if len(sys.argv) > 2 else 0
    device = sys.argv[3] if len(sys.argv) > 3 else "auto"

    # 0 means use all available cores
    if cpu_threads <= 0:
        cpu_threads = os.cpu_count() or 4

    print(f"Loading model (device={device}, cpu_threads={cpu_threads})...", file=sys.stderr)

    from faster_whisper import WhisperModel

    model = WhisperModel(
        "large-v3",
        device=device,
        compute_type="auto",
        cpu_threads=cpu_threads,
    )

    print(f"Transcribing {audio_path}...", file=sys.stderr)
    segments, info = model.transcribe(
        audio_path,
        language="ja",
        vad_filter=True,
    )
    print(
        f"Detected language: {info.language} (p={info.language_probability:.2f})",
        file=sys.stderr,
    )

    result = []
    for segment in segments:
        result.append(
            {
                "start": round(segment.start, 2),
                "end": round(segment.end, 2),
                "text": segment.text.strip(),
            }
        )

    print(f"Transcribed {len(result)} segments", file=sys.stderr)
    json.dump(result, sys.stdout, ensure_ascii=False)


if __name__ == "__main__":
    main()
