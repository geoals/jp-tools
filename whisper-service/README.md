# whisper-service

FastAPI transcription service. Loads a
[faster-whisper](https://github.com/SYSTRAN/faster-whisper) model at startup and
streams Japanese transcript segments back as NDJSON. Listens on port 8100.

## Endpoints

- `GET /health` — returns `{"status": "ok"}`.
- `POST /transcribe` — multipart upload of an audio file under the `audio`
  field. Streams one JSON object per segment (`start`, `end`, `text`) as NDJSON
  (`application/x-ndjson`). Pass `?words=true` to also get per-word timestamps.
  Transcription uses `language=ja` with VAD filtering. Client disconnect drops
  the generator and stops transcription.

## Configuration

Environment variables (with defaults):

- `WHISPER_MODEL_SIZE` — model to load (`large-v3`).
- `WHISPER_DEVICE` — `cuda` or `cpu` (`cuda`).
- `WHISPER_COMPUTE_TYPE` — compute precision (`auto`).

## Running

Directly:

```sh
uvicorn main:app --host 0.0.0.0 --port 8100
```

Via Docker Compose (bind-mounts the host HuggingFace cache so models aren't
re-downloaded):

```sh
docker compose -f docker-compose.gpu.yml up   # NVIDIA GPU
docker compose -f docker-compose.cpu.yml up   # CPU only
```
