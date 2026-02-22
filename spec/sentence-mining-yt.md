# Sentence Mining from YouTube

Automatic sentence mining pipeline: paste a YouTube URL, get transcribed
sentences, pick the ones worth studying, export to Anki.

## Problem

Manual sentence mining from YouTube is slow. You watch a video, hear a sentence
with an interesting word, pause, copy it somewhere, look up the word, format a
card, add it to Anki. Multiply by 20 sentences per video and it's tedious enough
that most people don't bother — or use tools that mine everything indiscriminately,
flooding Anki with low-value cards.

What's needed is a pipeline that handles the mechanical parts (transcription,
segmentation, dictionary lookup) and lets the human focus on the judgment call:
_is this sentence worth studying?_

## Workflow Overview

```
YouTube URL
│
▼
┌──────────┐ ┌───────────────┐ ┌──────────────────┐
│ Download │────▶│ Transcribe │────▶│ Segment into │
│ (yt-dlp) │ │ (faster- │ │ sentences │
│ │ │ whisper) │ │ (morphological │
└──────────┘ └───────────────┘ │ analysis) │
                                    └────────┬─────────┘
                                             │
                                             ▼
                                    ┌──────────────────┐
                                    │ Filter & score │
                                    │ (noise, quality, │
                                    │ frequency) │
                                    └────────┬─────────┘
                                             │
                                             ▼
                                    ┌──────────────────┐
                                    │ Present to user │
                                    │ (browse, filter, │
                                    │ select) │
                                    └────────┬─────────┘
                                             │
                                             ▼
                                    ┌──────────────────┐
                                    │ Export to Anki │
                                    │ (AnkiConnect) │
                                    └──────────────────┘
```

## MVP Phases

### MVP 1 — End-to-end pipeline

The smallest thing that's useful: paste a URL, see sentences, pick some, send to
Anki. No filtering, no dictionary lookup, no word selection.

**Scope:**
- Web UI: single input field + submit button
- Backend downloads audio via `yt-dlp`
- Transcribe with `faster-whisper` (called as subprocess)
- Segment whisper output into sentences using morphological analysis
- Display all sentences in the UI as a selectable list
- User checks the sentences they want
- Export selected sentences to Anki via AnkiConnect (sentence on front, empty back)

**What this validates:**
- The transcription quality is good enough for mining
- Sentence segmentation produces reasonable boundaries
- The end-to-end flow works and is faster than manual mining

**What it explicitly does NOT include:**
- No quality filtering or scoring
- No frequency-based filtering
- No target word selection or dictionary definitions
- No audio clips or screenshots
- No LLM explanations
- No integration with the vocabulary DB (known/unknown tracking)

---

### MVP 2 — Usable for daily mining

Make the output actually useful as Anki cards.

**Adds:**
- Target word selection: user clicks a word in the sentence to mark it as the
  target word
- Dictionary definition lookup from yomitan dictionary files for the target word
- Morphological analysis displayed per sentence (furigana, word boundaries)
- Basic quality filtering: drop too-short sentences (< 3 content words),
  too-long sentences (> 20 words), and noise patterns (music symbols ♪,
  repeated characters, sound effects)
- Name/proper noun detection via POS tags (固有名詞) — names are visually
  marked but not counted as "unknown" for filtering purposes
- Audio clip extraction: `ffmpeg` slices the audio at whisper timestamps for
  the selected sentence, attached to the Anki card

**Card format at this stage:**
- Front: sentence (with target word highlighted)
- Back: target word reading + dictionary definition + audio clip

---

### MVP 3 — Smart filtering

Help the user find the *right* sentences faster.

**Adds:**
- Frequency-based filtering: integrate a word frequency list; user sets a
  threshold (e.g. "hide sentences where all words are in the top 5000"); words
  below the threshold are highlighted as "interesting"
- Integration with jp-tools vocabulary DB: sentences containing words with
  status `unknown` are promoted; sentences with only `known` words are dimmed
- i+1 filtering: surface sentences with exactly 1 unknown word
- Sentence grouping: group sentences by their target-word candidates so the user
  can browse "all sentences containing 〜てしまう" together
- Sorting options: by timestamp (video order), by number of unknown words, by
  "interestingness" score

---

### Later — Polish

- LLM nuance/register field on the card (postponed per user request)
- Video frame capture for image context on cards
- Batch processing: queue multiple videos
- Processing status/progress UI (transcription can take minutes)
- Re-process with different filter settings without re-transcribing
- Persistent storage of transcription results (cache in SQLite)

## Component Details

### Download (yt-dlp)

Call `yt-dlp` as a subprocess to download audio.

- Extract audio only (`-x --audio-format wav`) — no need for video in MVP 1
- For MVP 2 (audio clips), keep the full audio file until export is done
- For later (video frames), download video too
- Store downloads in a temp directory, clean up after export

**Edge cases:**
- Playlists: reject, require single video URL
- Age-restricted: may fail, surface the error
- Very long videos (> 1 hour): warn the user about transcription time

### Transcription (faster-whisper)

faster-whisper is a Python library (CTranslate2-based, faster than vanilla
whisper). Since the server is Rust, call it as a subprocess.

**Integration approach:** A small Python script that:
1. Takes an audio file path as argument
2. Runs faster-whisper with `large-v3` model
3. Outputs JSON: list of segments with `{ start, end, text }`

```json
[
  { "start": 0.0, "end": 3.2, "text": "今日は皆さんにお知らせがあります" },
  { "start": 3.5, "end": 6.1, "text": "来週から新しいプロジェクトが始まります" }
]
```

The Rust server calls this script via `tokio::process::Command` and parses the
JSON output.

**Why subprocess over a Python sidecar service:** Simpler. Transcription is a
batch job triggered by user action, not a low-latency service. A subprocess
starts, does its work, exits. No process management needed.

**Model choice:** `large-v3` for Japanese. Smaller models have noticeably worse
accuracy for Japanese speech.

### Sentence Segmentation

Whisper segments are often phrases, not grammatical sentences. A whisper
"segment" might be half a sentence or two sentences merged.

**Strategy:**

1. Start with whisper segments as the initial split
2. Concatenate adjacent segments that look like they're part of the same
   sentence (heuristic: no sentence-ending particle or punctuation at the
   boundary)
3. Split segments that contain multiple sentences (detect via morphological
   analysis: look for sentence-ending forms like 終止形 followed by a new
   clause)

**Sentence-ending signals (from morphological analysis):**

- Tokens with 終止形 (terminal form) conjugation followed by a pause
- Sentence-ending particles: よ、ね、な、さ、ぞ、わ
- Punctuation: 。！？

This doesn't need to be perfect. Whisper's segmentation is already decent for
Japanese — the morphological refinement catches the worst splits.

### Quality Filtering (MVP 2+)

Heuristics to filter noise:

| Filter                        | What it catches                                  |
| ----------------------------- | ------------------------------------------------ |
| Too short (< 3 content words) | Fragments, greetings, interjections              |
| Too long (> ~20 words)        | Run-on transcription errors                      |
| Music/SFX patterns            | ♪, ～♪, repeated kana (あああ), brackets [拍手]  |
| Repeated text                 | Whisper hallucination (same phrase repeated)     |
| Low confidence                | If faster-whisper exposes per-segment confidence |

Filters are **soft by default** — filtered sentences are hidden but recoverable
(toggle "show filtered" in UI). The user always has the final say.

### Name Detection (MVP 2+)

Use POS tags from morphological analysis. Vibrato with UniDic tags proper nouns
as `名詞-固有名詞-*`.

Names are:

- Visually distinguished in the UI (different color or tag)
- Excluded from "unknown word" counts for frequency/i+1 filtering
- Not eligible as target words (you don't mine a name as vocabulary)

**Known limitation:** The morphological analyzer won't catch all names,
especially unusual ones or those written in hiragana. Acceptable for MVP —
misclassified names just show up as regular unknown words.

### Web UI

Minimal. Two views:

**Input view:**

- Text input for YouTube URL
- Submit button
- Status indicator while processing (downloading... transcribing... analyzing...)

**Results view:**

- List of sentences, each as a selectable card/row
- Checkbox to select/deselect
- Filter controls (MVP 2+): quality filter toggle, frequency threshold slider
- For MVP 2+: clicking a word in the sentence marks it as the target word
  (highlighted differently)
- Export button: sends selected sentences to Anki

**Technology:** Start with htmx + server-rendered HTML. The interaction pattern
(submit form, wait, render list, select items, submit again) maps well to htmx.
No need for a JS framework.

### Anki Export

Use AnkiConnect HTTP API (localhost:8765) — same as existing jp-tools
architecture.

**MVP 1 card format:**

- Deck: configurable (default: "Sentence Mining")
- Front: sentence text
- Back: (empty — user adds definitions manually)

**MVP 2 card format:**

- Front: sentence with target word in bold/highlighted
- Back: target word + reading + dictionary definition + audio clip

**Note type:** Create a custom note type "jp-tools-sentence" with fields:
`Sentence`, `TargetWord`, `Reading`, `Definition`, `Audio`, `Source`.

## Integration with Existing jp-tools

### What's reused

- **Axum server** — new routes added to existing server
- **Vibrato tokenizer** — already planned as the core tokenizer service
- **SQLite DB** — same database, potentially new tables
- **AnkiConnect integration** — same export mechanism as card mining
- **cards table** — mined sentences go through the same staging → export flow

### What's new

- **yt-dlp subprocess** — download management
- **faster-whisper subprocess** — transcription
- **Sentence segmentation logic** — merging/splitting whisper segments
- **Quality filtering heuristics**
- **The sentence mining UI** (new web pages/routes)

### New API routes

```
POST /mining/youtube          — submit URL, start processing
GET  /mining/jobs/:id         — poll processing status
GET  /mining/jobs/:id/results — get sentences
POST /mining/export           — export selected sentences to Anki
```

### Data model additions

Transcription results should be cached so re-filtering doesn't require
re-transcription:

```sql
CREATE TABLE mining_jobs (
    id INTEGER PRIMARY KEY,
    youtube_url TEXT NOT NULL,
    video_title TEXT,
    audio_path TEXT,              -- path to downloaded audio
    status TEXT NOT NULL DEFAULT 'pending',  -- pending | transcribing | done | error
    error_message TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE mining_sentences (
    id INTEGER PRIMARY KEY,
    job_id INTEGER NOT NULL REFERENCES mining_jobs(id),
    text TEXT NOT NULL,
    start_time REAL NOT NULL,     -- seconds
    end_time REAL NOT NULL,       -- seconds
    quality_score REAL,           -- 0.0-1.0, null if not scored
    is_filtered BOOLEAN NOT NULL DEFAULT FALSE,
    filter_reason TEXT,           -- why it was filtered (if applicable)
    created_at TEXT NOT NULL
);

CREATE INDEX idx_mining_sentences_job ON mining_sentences(job_id);
```

## Open Questions

### Sentence segmentation quality

How good is whisper's segmentation for Japanese out of the box? Need to test
with a few real videos before investing heavily in the morphological
merge/split logic. It may be good enough with minimal post-processing.

### faster-whisper output format

Does faster-whisper expose word-level timestamps or only segment-level?
Word-level timestamps would make audio clip extraction more precise. Need to
check the API.

### Yomitan dictionary file format

For MVP 2, we need to read definitions from yomitan dictionary files directly.
Format details TBD — user will provide information about the dictionary format.

### Frequency list choice

For MVP 3, need a frequency list. Candidates: BCCWJ, Innocent Corpus (novels),
or a media/spoken-language corpus. Decision deferred.

### Processing time UX

Transcribing a 10-minute video might take 30-60 seconds even with GPU. Options:

- Block the UI with a progress indicator (simplest)
- Background job with polling (more complex but better UX)

MVP 1 can block. Revisit if it feels too slow.
