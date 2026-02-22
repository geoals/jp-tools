# Features

Ordered by priority. Earlier features are prerequisites for later ones.

## Tier 1 — Foundation (must build first)

### Tokenizer Service

Wraps a morphological analyzer behind a clean internal API.

**Input:** raw Japanese text (sentence or paragraph)
**Output:** list of `{ surface, lemma, reading, pos }` tokens

**Feasibility:** Straightforward. LinDera (Rust-native) or MeCab (C++, FFI) are
both well-documented. Dictionary choice matters more than engine choice — UniDic
gives better lemmatization than IPAdic.

**Open decision:** Rust (LinDera/Vibrato) vs calling MeCab via FFI or subprocess.
See [architecture.md](./architecture.md).

---

### Knowledge DB + Import Pipeline

The vocabulary table and the tools to populate it.

- Anki CSV/apkg import
- Mass-read calibration
- Frequency list triage UI

**Feasibility:** Straightforward. The Anki import is the only fiddly part (note
type field detection). See [cold-start.md](./cold-start.md) for full details.

---

## Tier 2 — Daily-Use Reading Features

### Texthooker Page with Highlighting

A web page that receives text from Textractor (via clipboard or websocket),
tokenizes it, and highlights words by knowledge status.

**Color coding:**
- Known — no highlight (default text)
- Learning — subtle underline or light highlight
- Unknown — bold highlight (the words you should pay attention to)

Yomitan continues to work as the popup dictionary on top of this page.

**Feasibility:** High. Texthooker pages are simple HTML/JS apps. Adding
tokenization + DB lookup per sentence is cheap. The main question is latency —
needs to feel instant.

**Integration:** Text arrives via clipboard polling or websocket from Textractor.
Tokenize server-side (or via WASM in-browser), query vocabulary DB, return
annotated HTML.

---

### LLM Word Explanations

On-demand deep explanation of a word in context. Triggered manually (click/hotkey
on a highlighted word).

**Input to LLM:**
- Surface form, lemma, reading, POS (from morphological analyzer)
- The full sentence as context
- Optionally: 2-3 other recent sentences containing the same word

**What the LLM should provide:**
- Core meaning in this specific context
- How it differs from similar/synonymous words
- Register (formal/casual/written/spoken)
- Common collocations
- Cultural nuance where relevant
- Example of a common mistake learners make with this word

**Feasibility:** High — this is what LLMs excel at. Main considerations:
- Cache responses in `word_explanations` table to avoid repeat API calls
- Same word in very different contexts may need separate explanations
- Model choice: Claude or GPT-4 class models for quality; could use a smaller
  model for cost if quality is sufficient
- Latency: 2-5 seconds is acceptable for an on-demand feature

**Why morphological analysis matters here:** Without the lemma and POS from the
analyzer, the LLM has to guess which word you mean and which reading applies.
Feeding structured analyzer output makes the prompt precise and the response
reliable.

---

## Tier 3 — Media Mining

### VN Card Mining

When reading a VN with Textractor:
- One-click to mine current sentence as a flashcard candidate
- Automatically capture: sentence text, target word (the highlighted unknown),
  screenshot, audio clip
- Card goes to staging table for later review

**Feasibility:** High for text + screenshot. Audio capture depends on the VN
setup — some texthooker setups support audio export, others require
ShareX-style screen recording or game audio capture.

---

### YouTube Pipeline

1. Download video+audio via `yt-dlp`
2. Generate accurate subtitles via Whisper (large-v3 for Japanese)
3. Display in a custom player with highlighting
4. One-click mining: `ffmpeg` slices audio at subtitle timestamp, grabs a video
   frame, creates a card candidate

**Feasibility:** Medium-high.
- Whisper large-v3 is good for Japanese but requires a decent GPU for local
  inference (or use an API)
- Whisper occasionally hallucinates during silence or background music
- Timestamp alignment is usually good but not frame-perfect
- The custom video player is a meaningful UI effort

**Alternative:** Use existing tools (e.g. asbplayer) for the video player and
focus on the subtitle generation + knowledge-based filtering.

---

### Bulk i+1 Filtering

Given a set of subtitle lines or text passages:
- Tokenize each sentence
- Check all tokens against vocabulary DB
- Keep only sentences with exactly 1 unknown word (i+1)
- Optionally: also keep sentences with 1 unknown + 1 learning word
- Discard very short sentences (< 4 tokens) and very long ones (> 25 tokens)

**Feasibility:** High, once the knowledge DB is populated. This is a
straightforward DB query per sentence.

---

## Tier 4 — Nice to Have

### Vocabulary Statistics Dashboard

- Words known over time (graph)
- Breakdown by JLPT level or frequency band
- "Words learned this week/month"
- Most-encountered unknown words (high-value targets)

### Bulk Text Analysis

Paste or upload a text (article, book chapter). Get a report:
- % of words known
- List of unknown words sorted by frequency in the text
- Estimated difficulty level
- "If you learn these 15 words, you'll know 98% of this text"

### Grammar Pattern Tracking

Separate from vocabulary. Track which grammar constructions you've encountered
and studied. Harder to implement because grammar patterns don't tokenize cleanly.
Deferred.
