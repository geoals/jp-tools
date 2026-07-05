# manga-ocr-service

Local OCR service for `manga-mine`: a thin FastAPI wrapper around
[kha-white's manga-ocr](https://github.com/kha-white/manga-ocr) (Transformer
trained on manga; handles vertical text, furigana, stylized fonts).

Recognition only — send a **pre-cropped text region** (one speech bubble),
not a full page.

## Setup

```sh
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
```

The model (~450 MB) downloads from Hugging Face on first startup. Runs on GPU
when available; set `MANGA_OCR_FORCE_CPU=1` to force CPU.

## Run

```sh
.venv/bin/uvicorn main:app --host 0.0.0.0 --port 8200
```

## API

- `GET /health` → `{"status": "ok"}`
- `POST /ocr` (multipart, field `image`) → `{"text": "認識されたテキスト"}`
