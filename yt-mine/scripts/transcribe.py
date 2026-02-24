#!/usr/bin/env python3
"""Transcribe an audio file using faster-whisper.

One-shot mode:
    python3 transcribe.py <audio_path>

Worker mode (persistent — model loaded once):
    python3 transcribe.py --worker
    Reads audio paths from stdin (one per line), writes JSON results to stdout.
    Prints READY to stdout after model is loaded.

Outputs JSON to stdout: [{"start": 0.0, "end": 3.2, "text": "..."}, ...]
All progress/status messages go to stderr so they appear in the server logs.
"""

import json
import sys


def load_model(device):
    print(f"Loading whisper large-v3 speech-to-text model (device={device})...", file=sys.stderr)

    from faster_whisper import WhisperModel

    return WhisperModel(
        "large-v3",
        device=device,
        compute_type="auto",
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


def run_oneshot(args, device):
    if len(args) < 1:
        print("Usage: transcribe.py <audio_path>", file=sys.stderr)
        sys.exit(1)

    audio_path = args[0]
    model = load_model(device)
    result = transcribe_audio(model, audio_path)
    json.dump(result, sys.stdout, ensure_ascii=False)


def run_worker(device):
    model = load_model(device)

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

    disable_cuda = "--disable-cuda" in args
    args = [a for a in args if a != "--disable-cuda"]
    device = "cpu" if disable_cuda else "cuda"

    if len(args) > 0 and args[0] == "--worker":
        run_worker(device)
    else:
        run_oneshot(args, device)


if __name__ == "__main__":
    main()
