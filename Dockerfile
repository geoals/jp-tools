# Whisper transcription environment with CUDA support.
# Only used for running faster-whisper — the Rust server runs on the host.
#
# Build once:  docker compose up -d
# Then run:    JP_TOOLS_TRANSCRIBE_SCRIPT=scripts/transcribe-docker.sh cargo run
FROM nvidia/cuda:12.4.1-cudnn-runtime-ubuntu22.04

RUN apt-get update && apt-get install -y --no-install-recommends \
    python3 \
    python3-pip \
    ffmpeg \
    && rm -rf /var/lib/apt/lists/*

COPY requirements.txt .
RUN pip3 install --no-cache-dir -r requirements.txt

WORKDIR /app
COPY scripts/transcribe.py scripts/

CMD ["sleep", "infinity"]
