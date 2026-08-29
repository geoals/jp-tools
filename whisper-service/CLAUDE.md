# whisper-service

@README.md

## The model and the prompt go together

`large-v3-turbo` is what the compose files run: several times faster than
large-v3 and small enough in VRAM to transcribe while a game holds the rest of
the card, which large-v3 is not.

Left alone it returns Japanese with **no 。 or 、 at all**, and a mined
sentence without them is not a sentence. Whisper punctuates by imitating what
it was primed with, so `WHISPER_INITIAL_PROMPT` carries a punctuated Japanese
sample. Quantization is not the lever — `int8_float16` punctuates no better.

The prompt also makes segments longer, so a segment holds more than one sentence
far more often than large-v3's do.
