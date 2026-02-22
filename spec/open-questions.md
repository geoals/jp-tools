# Open Questions

Decisions that need research or deliberation before implementation.

## Morphological Analysis

### Which dictionary?

UniDic is recommended for better lemmatization, but it comes in several versions:
- **UniDic (full)** — ~500MB, includes accent data
- **unidic-lite** — smaller, fewer features
- **UniDic-CWJ** (Contemporary Written Japanese) — optimized for modern text

Need to test whether UniDic's lemmatization is actually better than IPAdic's for
the specific use case of vocabulary DB lookups. If 食べさせられた → 食べる works
correctly with both, the simpler IPAdic might be fine.

### Compound word handling

MeCab/LinDera tokenize 東京都 as a single token, but 取り消す might become
取り + 消す or stay as 取り消す depending on the dictionary. How should the
knowledge DB handle this?

Options:
- Store whatever the tokenizer outputs and accept inconsistency
- Normalize to longest-match compounds
- Store both the compound and its parts, with a relationship between them

### Katakana loanwords

Should コンピューター be tracked as vocabulary? Most learners know these already.
Options:
- Auto-mark all katakana-only words as known during calibration
- Include them but let the frequency triage handle it
- Ignore them entirely in highlighting

## LLM Integration

### Which model for word explanations?

Quality vs. cost tradeoff. Need to test:
- Claude Sonnet — good quality, moderate cost
- Claude Haiku — cheaper, may be sufficient
- GPT-4o-mini — another cheap option

Run the same 50 word explanations through each and compare quality. The
explanation feature is high-value enough that quality should win over cost.

### Prompt engineering

The word explanation prompt needs careful design. Questions:
- How much context to include? (just the sentence, or surrounding sentences too?)
- Should the explanation be in English, Japanese, or mixed?
- How to handle words where the "interesting" part is cultural, not linguistic?
- Should the response format be structured (JSON) or free-form (markdown)?

### Caching granularity

Same word, different sentences — when do we need a fresh explanation?
- 上がる in "血圧が上がった" vs "2階に上がった" genuinely needs different
  explanations (rise vs. go up)
- 食べる in "ご飯を食べた" vs "パンを食べた" doesn't

Could use the LLM itself to determine if a cached explanation applies to a new
context, but that adds another API call.

## Yomitan

### Is the lack of lookup tracking actually a problem?

Option D (minimal integration) means we don't know which words you looked up in
Yomitan. In practice, is this data valuable? If you look up a word, you
probably either:
1. Already know it and were just checking → no action needed
2. Don't know it and want to mine it → you'll use our mining UI

The lookup event itself may not carry much signal. Test this assumption before
building complex integration.

## YouTube Pipeline

### Local Whisper vs API?

- Local: free, private, but needs GPU (or is very slow on CPU)
- OpenAI Whisper API: $0.006/minute, fast, easy
- Groq Whisper API: free tier available, fast

For a personal tool, the API cost is negligible. But local gives you
independence.

### Is a custom video player worth building?

Existing tools:
- **asbplayer** — browser extension, loads external subtitles onto streaming
  video, supports Yomitan, has mining features
- **mpv + mpvacious** — desktop player with Anki mining

If asbplayer or mpv already handles the video playback + mining, our tool only
needs to provide: (1) better subtitles via Whisper, and (2) i+1 filtering of
subtitle files before loading them into the player. Building a custom player
may be unnecessary.

## Scope & Priorities

### What's the MVP?

The minimum useful tool is:
1. Tokenizer + vocabulary DB
2. Anki import + frequency list triage (cold start)
3. Texthooker page with highlighting

Everything else (LLM explanations, YouTube pipeline, card mining) builds on top.

### Should this be one monolith or multiple small tools?

The architecture diagram shows a single API server, but some pieces (Whisper
transcription, bulk text analysis) could be standalone CLI tools that share the
same SQLite DB. A modular approach matches "small incremental work" — build
each piece as a focused tool, connected by the shared DB schema.
