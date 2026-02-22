#!/bin/bash
# Wrapper that runs transcribe.py inside the Docker container.
# Used as JP_TOOLS_TRANSCRIBE_SCRIPT when the host doesn't have CUDA/faster-whisper.
#
# One-shot: rewrites the host audio path to the container mount point (/app/audio/).
# Worker:   passes --worker through with -i to keep stdin open.

if [ "$1" = "--worker" ]; then
    shift
    # Rewrite host audio paths to container paths on stdin before forwarding.
    # The while-read loop reads from Rust's pipe, rewrites each path, and pipes
    # into docker exec. Docker's stdout flows back to Rust unchanged.
    while IFS= read -r line; do
        echo "/app/audio/$(basename "$line")"
    done | docker exec -i -w /app jp-tools-whisper python3 scripts/transcribe.py --worker "$@"
    exit
fi

AUDIO_HOST="$1"
shift

# Replace everything up to and including /audio/ with /app/audio/
AUDIO_CONTAINER="/app/audio/$(basename "$AUDIO_HOST")"

exec docker exec -w /app jp-tools-whisper python3 scripts/transcribe.py "$AUDIO_CONTAINER" "$@"
