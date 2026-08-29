"""
Whisper transcription service.

Loads a faster-whisper model at startup and exposes a streaming transcription
endpoint. Audio files are uploaded via multipart POST and results are streamed
back as NDJSON — one line per transcript segment.

Usage:
    uvicorn main:app --host 0.0.0.0 --port 8100
"""

import json
import os
import tempfile

from fastapi import FastAPI, UploadFile, File
from fastapi.responses import StreamingResponse
from faster_whisper import WhisperModel

MODEL_SIZE = os.environ.get("WHISPER_MODEL_SIZE", "large-v3")
DEVICE = os.environ.get("WHISPER_DEVICE", "cuda")
COMPUTE_TYPE = os.environ.get("WHISPER_COMPUTE_TYPE", "auto")

# Whisper punctuates by imitation, not by rule: it continues the style of the
# text it is primed with. large-v3-turbo left to itself returns Japanese with
# no 。 or 、 at all, which costs a mined sentence its shape. Priming it with a
# punctuated Japanese sample is what puts them back.
INITIAL_PROMPT = os.environ.get(
    "WHISPER_INITIAL_PROMPT",
    "以下は日本語の書き起こしです。句読点を正しく付けます。"
    "今日は、いい天気ですね。ええ、そうですね。",
)

app = FastAPI()
model: WhisperModel | None = None


@app.on_event("startup")
def load_model():
    global model
    print(f"Loading whisper model: {MODEL_SIZE} (device={DEVICE}, compute={COMPUTE_TYPE})")
    cpu_threads = os.cpu_count() or 4
    model = WhisperModel(MODEL_SIZE, device=DEVICE, compute_type=COMPUTE_TYPE, cpu_threads=cpu_threads)
    print("Model loaded, ready for requests")


@app.get("/health")
def health():
    return {"status": "ok"}


@app.post("/transcribe")
async def transcribe(audio: UploadFile = File(...), words: bool = False):
    suffix = os.path.splitext(audio.filename or "audio.wav")[1]
    with tempfile.NamedTemporaryFile(delete=False, suffix=suffix) as tmp:
        tmp.write(await audio.read())
        tmp_path = tmp.name

    def generate():
        try:
            segments, _info = model.transcribe(
                tmp_path,
                language="ja",
                vad_filter=True,
                word_timestamps=words,
                initial_prompt=INITIAL_PROMPT or None,
            )
            for segment in segments:
                payload = {"start": round(segment.start, 2), "end": round(segment.end, 2), "text": segment.text.strip()}
                if words:
                    payload["words"] = [
                        {"word": w.word, "start": round(w.start, 2), "end": round(w.end, 2)}
                        for w in segment.words
                    ]
                line = json.dumps(payload, ensure_ascii=False)
                yield line + "\n"
        finally:
            os.unlink(tmp_path)

    # StreamingResponse stops iterating the generator on client disconnect,
    # which drops it and stops transcription — no explicit handling needed.
    return StreamingResponse(generate(), media_type="application/x-ndjson")
