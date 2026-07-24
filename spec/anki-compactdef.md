# CompactDef — a 2-second backside gloss for fast recognition review

## The problem

Reviews take too long per card (12–14s average; the goal is 4–5s). The load is
about to climb: ~20 new cards/day, reviews heading past 100–120/day. For that to
be sustainable each card has to be near-instant, which means training *instant
recognition*, not 8-second effortful recall.

Several causes, separated because they have separate fixes:

1. **Slow backside glance.** The habit is to glance at the back for ~0.5s to
   confirm a recollection even on a pass. Older cards had a short English gloss
   → glanceable in 0.5s. Newer cards have an empty `VocabDef` and only the full
   monolingual `VocabDefFull` (Sankoku, measured at 200–540 chars on recent
   notes) → the glance costs 2–3s. Confirmed in the collection: recent
   "Japanese sentences" notes have `VocabDef` empty and `VocabDefFull` long.

2. **Decision-during-review friction.** The real time sink on the worst cards
   isn't reading — it's *evaluating the card mid-review*: "is this sentence any
   good? should I delete it? reformulate it?" This breaks rhythm far worse than
   a 2s read. Fix: get the decision out of the loop entirely (see below).

3. **Card quality / selection.** Too many cards have sentences that are too
   long, contain more than one unknown word, use the word in a non-canonical
   (metaphor / wordplay) way, or aren't 100% understood. These are the cards
   that go fuzzy at long intervals.

This doc addresses (1) with a new **CompactDef** field, generated at mine-time
and backfilled over the collection, and lays the groundwork for (3) with a
poor-quality-card triage skill. (2) is a review-workflow change, noted below.

## The review-workflow change (no code)

During reviews: two verdicts (pass/fail) plus **one instant "bad card" keypress**
(a flag or a `reformulate` tag) hit *without deliberating*. Never delete or edit
mid-review. Process the flagged pile offline once a week, where agonizing is
cheap. This converts the worst 15–30s contemplation cards into 1s cards and is
probably the single biggest lever on the average. The triage skill (below) can
pre-populate that flagged pile.

## CompactDef: what it is

A new field on the "Japanese sentences" note type, rendered at the **top** of the
card back, above the existing `VocabDef`/`VocabDefFull` block. It holds the
specific sense the target word carries **in that card's sentence**, compressed to
be readable in **under 2 seconds** — at ~14k chars/hour that is roughly **8
Japanese characters**, or a few English words. It is a pass/fail *gate*, not a
place to learn nuance; the full definitions stay right below it for when a card
is failed and actually needs studying.

### Content rules (the prompt encodes these)

- **Mostly Japanese, English when English is safe.** Default to Japanese. Use
  English only when the word has a clear, direct English counterpart that can't
  mislead — concrete nouns, established science/technical concepts (焼却炉 →
  "incinerator", 要人 → "VIP / dignitary"). The bar: *could writing it in
  English give off a wrong meaning or nuance?* If yes → Japanese.
- **Synonyms vs. a short phrase.**
  - If the word is genuinely interchangeable with 1–2 close synonyms, give those
    synonyms (弱る・衰える).
  - If the word has a nuance its near-neighbours don't, **do not give synonyms** —
    a synonym would reinforce a wrong equivalence. Give a short plain-Japanese
    phrase pinning the specific sense instead. Onomatopoeia especially: they
    usually carry a specific feel, so describe it rather than equate it.
- **Kanji compound → 和語 counterpart.** When the word is essentially the
  Sino-Japanese (音読み) form of a plain verb/act, give the native 和語 form:
  奪取 → 奪い取る, 減退 → 弱る・衰える. (Reading of 奪取 is だっしゅ; 奪い取る is
  うばいとる — the same act, so the 和語 is the fastest possible gloss.)
- **Sense-in-context.** Gloss the meaning as used *in this sentence*, not the
  dictionary's first sense.
- **Length.** Target under 8 Japanese characters (or 2–4 English words); up to
  ~12 is acceptable when a word genuinely needs it to be understandable. Never a
  full sentence. No trailing punctuation, no labels, no romaji, no markdown.

### Prompt (used verbatim in code and skills)

System prompt:

```
You write an ultra-short gloss ("CompactDef") for a Japanese vocab flashcard.
It sits at the top of the card back and must be readable in under 2 seconds —
about 8 Japanese characters, or a few English words. It is a quick check of
recognition, not a full definition (the full dictionary entry is shown below
it). Gloss the sense the word carries IN THE GIVEN SENTENCE.

Rules:
- Default to Japanese. Use English ONLY when the word has a clear, direct
  English counterpart that cannot mislead — concrete nouns and established
  technical/scientific terms (e.g. 焼却炉 → incinerator). Ask yourself: could
  the English give a wrong nuance? If yes, use Japanese.
- If the word is freely interchangeable with one or two close synonyms, give
  those synonyms (e.g. 弱る・衰える).
- If the word carries a nuance its near-synonyms do NOT share, do NOT give a
  synonym — it would reinforce a wrong equivalence. Give a short plain-Japanese
  phrase that pins the specific sense. This applies especially to onomatopoeia,
  which usually have a specific feel rather than a synonym.
- If the word is essentially the Sino-Japanese (音読み) compound form of a plain
  act, give its native 和語 counterpart (e.g. 奪取 → 奪い取る, 減退 → 衰える).
- Output ONLY the gloss. No labels, no quotes, no romaji, no markdown, no
  trailing punctuation. Target under 8 Japanese characters (up to ~12 only when
  the word genuinely needs it); never a full sentence.
```

User message:

```
Word: {word}
Sentence: {sentence}
```

Model: cheap/fast is fine (this is a short lookup). read-stats already defaults
`JP_TOOLS_LLM_MODEL` to `claude-haiku-4-5`; CompactDef reuses it.

## Where it plugs in

Two card-creation paths exist; they inject CompactDef in different places.

### Daily driver: VN reading → Yomitan → read-stats `/anki-proxy` (DONE)

The daily flow reads a VN over Moonlight on the phone, Yomitan scans the line
feed, and **Yomitan creates the card**, POSTing `addNote` to read-stats'
`/anki-proxy`, which forwards to Anki. Rust never builds these notes, so
CompactDef can't be added at build time — it is added *after* the note exists:

1. The proxy forwards the `addNote` byte-for-byte (its existing contract — it
   never alters the forwarded request).
2. In the background, once Anki has assigned a note id, it:
   - generates CompactDef from the note's word + sentence and writes it with
     `updateNoteFields`;
   - fires `vn-capture.sh` to attach audio + picture (best-effort — a stale ring
     buffer or missing audio just skips media; CompactDef still lands).

This is why CompactDef is owned by the proxy and **not** by `vn-capture.sh`:
capture aborts early exactly in the no-audio case, but CompactDef must always be
written. It also folds the old "add card, then press the mine button" into one
action.

### yt-mine (TODO — not yet built)

yt-mine builds the note in Rust (`jp-mine-core::export` + `yt-mine` export
handler) and already has an `LlmDefiner`/`llm_definition` path for a *different*,
longer field. Adding CompactDef here means:

- add a `field_compact_def: Option<String>` to `jp_mine_core::config::AnkiConfig`
  and a `compact_def` to `NoteData`/`ExportSentence`, wired through
  `build_add_note_request` (mirror the existing `llm_definition` plumbing);
- add a `CompactDefiner` call (or extend `LlmDefiner`) using the prompt above,
  invoked in `yt-mine`'s export handler alongside the existing `define` call.

Deferred by request. Do NOT reuse the `llm_definition` / `LLMDef` field — that is
a separate, longer explanation field; CompactDef is its own field.

## Companion skills (prompt bases)

Both are collection-wide LLM passes over AnkiConnect. They share the note layout
below so the model judges the card as it actually renders.

### Note layout (feed to both skills as context)

- Note type: **Japanese sentences**. Relevant fields: `VocabKanji` (target word,
  dictionary form), `SentKanji` (the sentence, target word wrapped in `<b>`),
  `VocabDef` (short def — usually empty on recent cards), `VocabDefFull` (full
  monolingual, Sankoku), `VocabFurigana`, `VocabPitchNum`, `Frequency`,
  `Document` (source), `Image`, `SentAudio`/`VocabAudio`, and the new
  `CompactDef`.
- Front shows `VocabKanji` big, then `Hint` (if any), then `SentKanji` with the
  target word pitch-coloured. Back shows furigana headword, sentence + audio,
  then `CompactDef` (new, top), then `VocabDef` else `VocabDefFull`, then image,
  frequency, source.

### Skill A — backfill CompactDef

Walk the deck via AnkiConnect (`findNotes deck:... → notesInfo`), and for every
note whose `CompactDef` is empty, generate it with the prompt above from
`VocabKanji` + `SentKanji` and write it with `updateNoteFields`. Idempotent (skip
non-empty). Dry-run to a review file before writing; batch politely. Old and new
cards then converge on the same gloss style.

### Skill B — poor-quality-card triage

Evaluate each card against the quality bar and flag (never auto-delete) the bad
ones with a `reformulate` tag for offline review. Consider **all** fields and the
layout above. Primary criteria, in order:

1. **Sentence too long** — can't be read in ~2s (rough proxy: length well above
   the deck norm; recent good cards sit ~14–30 chars).
2. **Poor illustration of the word** — the sentence doesn't show the word's
   canonical use: metaphor / wordplay / idiom obscuring the plain sense, the
   target word barely load-bearing, or a non-representative sense.
3. **More than one likely-unknown word** — approximated by rare-word density
   (multiple low-`Frequency` / off-list tokens besides the target). This is a
   proxy, not truth, since the real "known words" set isn't available — flag as a
   hint, don't trust it as fact.

Output per flagged card: note id, the failing criteria, and a one-line reason.
Assist-only: it feeds the offline flagged pile; the human decides delete vs.
reformulate vs. keep.

## Status

- [x] Design + finalized prompt (this doc)
- [x] `read-stats/src/compactdef.rs` — the LLM call
- [x] `/anki-proxy` enrichment: CompactDef write + auto vn-capture on `addNote`
- [ ] Add `CompactDef` field to the note type + back template (via AnkiConnect,
      needs Anki running & a full sync — user's call)
- [ ] yt-mine CompactDef wiring (TODO above)
- [ ] Backfill skill (Skill A)
- [ ] Poor-quality-card triage skill (Skill B)
