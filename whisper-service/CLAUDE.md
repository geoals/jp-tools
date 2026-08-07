# whisper-service

@README.md

## The model and the prompt go together

`large-v3-turbo` is the default: 26x realtime on the GPU against large-v3's
10.7x, in 1188 MiB against 2070 MiB — little enough to transcribe while a game
holds the rest of the card, which large-v3 could not.

Left alone it returns Japanese with **no 。 or 、 at all**, and a mined
sentence without them is not a sentence. Whisper punctuates by imitating what
it was primed with, so `WHISPER_INITIAL_PROMPT` carries a punctuated Japanese
sample. Measured over the same two minutes of speech: 0.0% punctuation without
it, 7.7% with, against large-v3's 7.2%. Quantization is not the lever —
`int8_float16` gives the identical 0.0%.

The prompt also makes segments longer: 30% of them hold more than one sentence,
against 3% for large-v3.
