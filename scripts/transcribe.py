#!/usr/bin/env python3
"""Transcribe an audio file using faster-whisper.

Usage: python3 transcribe.py <audio_path>

Outputs JSON to stdout: [{"start": 0.0, "end": 3.2, "text": "..."}, ...]
"""

import json
import sys


def main():
    if len(sys.argv) != 2:
        print("Usage: transcribe.py <audio_path>", file=sys.stderr)
        sys.exit(1)

    audio_path = sys.argv[1]

    from faster_whisper import WhisperModel

    model = WhisperModel("large-v3", device="auto", compute_type="auto")
    segments, _info = model.transcribe(
        audio_path,
        language="ja",
        vad_filter=True,
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

    json.dump(result, sys.stdout, ensure_ascii=False)


if __name__ == "__main__":
    main()
