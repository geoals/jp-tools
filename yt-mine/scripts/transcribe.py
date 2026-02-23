#!/usr/bin/env python3
"""Transcribe an audio file using faster-whisper.

One-shot mode:
    python3 transcribe.py <audio_path> [cpu_threads] [device]

Worker mode (persistent — model loaded once):
    python3 transcribe.py --worker [cpu_threads] [device]
    Reads audio paths from stdin (one per line), writes JSON results to stdout.
    Prints READY to stdout after model is loaded.

Outputs JSON to stdout: [{"start": 0.0, "end": 3.2, "text": "..."}, ...]
All progress/status messages go to stderr so they appear in the server logs.
"""

import json
import os
import sys


def resolve_cpu_threads(cpu_threads):
    if cpu_threads <= 0:
        return os.cpu_count() or 4
    return cpu_threads


def load_model(cpu_threads, device):
    print(f"Loading whisper large-v3 speech-to-text model (device={device}, cpu_threads={cpu_threads})...", file=sys.stderr)

    from faster_whisper import WhisperModel

    return WhisperModel(
        "large-v3",
        device=device,
        compute_type="auto",
        cpu_threads=cpu_threads,
    )


def transcribe_audio(model, audio_path):
    """Transcribe a single audio file. Returns list of segment dicts."""
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
    return result


def run_oneshot(args):
    if len(args) < 1:
        print("Usage: transcribe.py <audio_path> [cpu_threads] [device]", file=sys.stderr)
        sys.exit(1)

    audio_path = args[0]
    cpu_threads = int(args[1]) if len(args) > 1 else 0
    device = args[2] if len(args) > 2 else "auto"
    cpu_threads = resolve_cpu_threads(cpu_threads)

    model = load_model(cpu_threads, device)
    result = transcribe_audio(model, audio_path)
    json.dump(result, sys.stdout, ensure_ascii=False)


def run_worker(args):
    cpu_threads = int(args[0]) if len(args) > 0 else 0
    device = args[1] if len(args) > 1 else "auto"
    cpu_threads = resolve_cpu_threads(cpu_threads)

    model = load_model(cpu_threads, device)

    print("READY", flush=True)

    for line in sys.stdin:
        audio_path = line.strip()
        if not audio_path:
            continue

        try:
            result = transcribe_audio(model, audio_path)
            json.dump(result, sys.stdout, ensure_ascii=False)
        except Exception as e:
            print(f"Transcription error: {e}", file=sys.stderr)
            json.dump({"error": str(e)}, sys.stdout, ensure_ascii=False)

        sys.stdout.write("\n")
        sys.stdout.flush()


def main():
    args = sys.argv[1:]

    if len(args) > 0 and args[0] == "--worker":
        run_worker(args[1:])
    else:
        run_oneshot(args)


if __name__ == "__main__":
    main()
