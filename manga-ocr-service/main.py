"""
Manga OCR service.

Loads kha-white's manga-ocr model at startup and exposes a one-shot OCR
endpoint. Image crops are uploaded via multipart POST; the recognized text
comes back as JSON.

The model does recognition only — it expects a pre-cropped text region
(a speech bubble or caption), not a full page.

Usage:
    uvicorn main:app --host 0.0.0.0 --port 8200
"""

import io
import os

from fastapi import FastAPI, File, UploadFile
from manga_ocr import MangaOcr
from PIL import Image

FORCE_CPU = os.environ.get("MANGA_OCR_FORCE_CPU", "") in ("1", "true")

app = FastAPI()
mocr: MangaOcr | None = None


@app.on_event("startup")
def load_model():
    global mocr
    print(f"Loading manga-ocr model (force_cpu={FORCE_CPU})")
    mocr = MangaOcr(force_cpu=FORCE_CPU)
    print("Model loaded, ready for requests")


@app.get("/health")
def health():
    return {"status": "ok"}


@app.post("/ocr")
async def ocr(image: UploadFile = File(...)):
    img = Image.open(io.BytesIO(await image.read())).convert("RGB")
    text = mocr(img)
    return {"text": text}
