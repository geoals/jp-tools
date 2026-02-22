#!/bin/bash
# Wrapper that runs transcribe.py inside the Docker container.
# Used as JP_TOOLS_TRANSCRIBE_SCRIPT when the host doesn't have CUDA/faster-whisper.
exec docker exec -w /app jp-tools-whisper python3 scripts/transcribe.py "$@"
