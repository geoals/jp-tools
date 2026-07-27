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

A field on the "Japanese sentences" note type, rendered at the **top** of the
card back, above the existing `VocabDef`/`VocabDefFull` block. It holds the
specific sense the target word carries **in that card's sentence**, written in
**English** so it is instant for a native English speaker to read, plus a usage
note where the word warrants one and a register tag. The learner can skim ~2
English sentences in the time 6 Japanese characters take, so the budget is
generous; the full Japanese definitions stay below it for cards that are failed
and actually studied.

> **Design history.** CompactDef started as an *ultra-short Japanese* gloss (~8
> chars). Two problems killed that: a Japanese gloss can itself contain a word
> the learner doesn't know (unknowable in advance), and — for a native English
> speaker whose goal is to *maximize immersion-reading time* — the fastest,
> most reliable recognition gate is English. The Japanese-acquisition load is
> carried by the reading itself and by the monolingual `VocabDefFull` below, not
> by this gate. So CompactDef is now English-first and richer.

### Content rules (the prompt encodes these)

- **English, nuance over translation.** Give a bare one/two-word translation
  **only** for a concrete term with a clean 1-to-1 equivalent (焼却炉 →
  "incinerator", 額縁 → "picture frame"). For anything with nuance, use a short
  phrase that carries the actual sense — never flatten it to one misleading word.
- **Sense-in-context.** Gloss the meaning as used *in this sentence*, not the
  dictionary's first sense.
- **Usage note (only when it earns one).** One short sentence capturing what a
  bare gloss misses: a fixed collocation ("almost always 与しやすい相手／者"), a
  polarity restriction (only in the negative; always pejorative), the typical
  speaker/situation, or a notable reference. Citing the Japanese word or its
  usual phrase here is fine and useful (this is *not* circular — the meaning line
  already carries the sense in English).
- **Two-axis tag line (always).** After the meaning/usage, on its own final line,
  emit `FAMILIARITY · FLAVOR[ · FLAVOR2[ · FLAVOR3]]` plus an optional trailing
  structural parenthetical. **Familiarity** (exactly one — recognizability on
  sight, population-wide): `CORE`/`COMMON`/`UNCOMMON`/`RARE`/`OBSCURE`. **Flavor**
  (1–3 — a production / "which room" warning): one baseline formality
  `SLANG`/`PLAIN`/`FORMAL`/`LITERARY` plus marks `TECHNICAL`/`RELIGIOUS`/
  `HONORIFIC`/`HUMBLE`/`DIALECT`/`ARCHAIC`/`VULGAR`/`DEROGATORY`/`CHILDISH`. Tag
  the in-sentence sense; usage overrides etymology. (Superseded the old single
  `EVERYDAY`/`PASSIVE`/`FORMAL-LITERARY`/`SPECIALIZED-DATED`/`OBSCURE` register;
  the reader-explain path in `services/llm.rs` uses this same two-axis system.)
- **Length & form.** Up to ~2 short English sentences plus the tag line. Any
  Japanese reading cited must be hiragana, never romaji. `clean_gloss` joins the
  lines with `<br>` so the tag line renders on its own line.

### Prompt (used verbatim in code and skills)

The **FAMILIARITY and FLAVOR rubric blocks are the single source of truth in
`read-stats/src/services/tags.rs`** (`FAMILIARITY_RUBRIC` / `FLAVOR_RUBRIC`);
both live LLM calls — `services/compactdef.rs` (this gloss) and
`services/llm.rs` (reader explain) — build their system prompts from those
consts, so the two can no longer drift. Keep the text below in sync with
`tags.rs`. The **FAMILIARITY definitions are the
sharpened set**: the axis turns on the single question "can you be certain EVERY
native adult recognizes it?", COMMON vs UNCOMMON split on active-vs-passive
vocabulary, RARE = the first tier where universal recognition can't be assumed
(A/B-tested against the old count-based wording; the sharpened set moved
borderline literary/idiom words to more defensible tiers — 逼迫 COMMON→UNCOMMON,
魑魅魍魎 UNCOMMON→RARE, 手向ける→UNCOMMON — with no regression once 手向ける's
elementary-dict presence was accounted for).

System prompt:

```
You write a compact ENGLISH gloss ("CompactDef") for a Japanese vocab flashcard.
It sits at the top of the card back as a fast recognition check; the full
Japanese dictionary entry is shown below it. The learner is a native English
speaker. Gloss the sense the word carries IN THE GIVEN SENTENCE.

Output exactly two lines and nothing else — no preamble, no markdown, and never
an XML or HTML tag of your own (do NOT write <meaning>, </meaning>, <usage>,
<br>, or any angle-bracket tag or label):
- Line 1 — the meaning, optionally followed by ". " and one short usage note.
- Line 2 — FAMILIARITY · FLAVOR[ · FLAVOR2[ · FLAVOR3]][ (structural)]

MEANING/USAGE: nuance-carrying English. A bare one/two-word translation ONLY for
a concrete 1-to-1 term (焼却炉 → incinerator); otherwise a short phrase that
carries the actual nuance. Optionally one short usage note — a fixed collocation,
a polarity restriction, or the typical speaker — where citing the Japanese word
or its usual phrase is fine. Adult/explicit words: gloss clinically. Any Japanese
reading you cite: hiragana, never romaji.

FAMILIARITY (exactly one) — recognition-on-sight across the native adult
population (NOT frequency, NOT whether they say it). The axis turns on ONE
question: can you be certain EVERY native adult recognizes it?
- CORE — every native, from childhood.
- COMMON — every native adult knows it, and for most it is ACTIVE vocabulary
  (they would use it themselves).
- UNCOMMON — essentially every native adult still RECOGNIZES it, but for a large
  portion it is PASSIVE only (known, but they would not produce it).
- RARE — the first tier where you CANNOT be certain every adult knows it. Many
  do, but a large share of such words are recognized mainly by people who read.
- OBSCURE — you can assume non-readers do NOT know it, and even among active
  readers only a portion recognize it.
A transparent compound of common parts with a predictable meaning (等価値 =
等価+価値) is understood first-encounter → COMMON or higher. Spoken/colloquial
words are more familiar than their rarity in writing suggests; don't demote them
for being informal.

FLAVOR (1-3) — if you SAY it in the wrong room, how do you sound. Emit exactly
one baseline formality, then add marks only when they carry an independent,
equally-important warning:
- baseline: SLANG / PLAIN (safe anywhere — always shown) / FORMAL (stiff if
casual; fine in formal speech or writing) / LITERARY (writing-only; theatrical
if spoken).
- marks: TECHNICAL, RELIGIOUS, HONORIFIC, HUMBLE, DIALECT, ARCHAIC, VULGAR,
DEROGATORY, CHILDISH.
Tag the IN-SENTENCE sense; other senses don't count (joking 成仏 = PLAIN, not
RELIGIOUS). A word can be marked in origin but plain in use — tag current usage,
not etymology.

STRUCTURAL (optional trailing parenthetical, orthogonal): (idiom) (mimetic)
(fixed phrase) (proverb) (name) (four-char idiom).

Judge from the word, the sentence, and your own knowledge ALONE — no frequency
data, no dictionary tags. No preamble, no markdown.
```

User message:

```
Word: {word}
Sentence: {sentence}
```

**Postclean (`services/compactdef.rs::clean_gloss`).** The model returns a short English
meaning/usage line and the two-axis tag line below it. `clean_gloss` strips any
literal `<meaning>`/`<usage>` placeholder tags the model echoes (Opus 5 does this),
trims each line, strips stray wrapping quotes, drops blank lines, and joins the
rest with `<br>` so the tag line renders on its own line in Anki's HTML (plain
newlines don't render). The caller skips writing an empty result.

Model: **pinned to `claude-opus-5`** in `services/compactdef.rs`, with thinking disabled
and low effort. The tag axes were shown to need no thinking and no external
signals; opus ≈ sonnet on tags but opus is preferred for the meaning/usage prose
(see *Why no external signals* below). Both live LLM calls — this one and the
reader-explain path in `services/llm.rs` — are pinned to opus-5 with thinking off and no
longer read `JP_TOOLS_LLM_MODEL` (that env var now only configures yt-mine's
definer). The hand backfill used this same prompt.

### Why no external signals (experiment log)

Settled by experiment against hand-graded gold (tested on sonnet-5 and opus-4-8);
recorded so nobody re-tries these:

- **BCCWJ frequency as input → HURT familiarity.** A rank anchors the model toward
  "catalogued/known" and inflates familiarity; it demoted rare literary words
  (恥垢, 手向ける) and helped none.
- **Thinking / high effort → HURT the tag axes.** Adaptive/high-effort reasoning
  talks itself *up* the familiarity scale ("an educated adult would know this").
  No-think beat think on both models; opus ≈ sonnet. This is why both live calls
  disable thinking.
- **jitendex `rare`/`dated` flag → useless for familiarity.** ~Zero coverage on
  the words that actually miss (JMdict `rare` tags spellings, not "rare word").
- **sankoku `〔文〕` (literary) as a familiarity nudge → HURT.** `〔文〕` is register,
  not familiarity (忖度 is `〔文〕` yet COMMON); feeding it inflated 齟齬/逗留 upward.
- **jitendex/sankoku candidate tags for FLAVOR → NET ZERO.** Helped one word
  (精進, dropped a wrong RELIGIOUS), hurt one (仰せ, anchored into dropping a correct
  ARCHAIC); the model avoids the wrong-sense traps (布石, 寸法) unaided.

Conclusion: the anchoring effect is axis-general — any external metadata nudges the
model toward "known/standard." The residual errors (RARE↔UNCOMMON on literary
words; the PLAIN/FORMAL/LITERARY baseline) are inherent gray zones the golds
themselves are debatable on, not signal-fixable. So: model judgment only, no
external signals, thinking off.

## Where it plugs in

Two card-creation paths exist; they inject CompactDef in different places.

### Daily driver: VN reading → Yomitan → read-stats `/anki-proxy` (DONE)

The daily flow reads a VN with `#read` open beside it, Yomitan scans the line
feed, and **Yomitan creates the card**, POSTing `addNote` to read-stats'
`/anki-proxy`, which forwards to Anki. Rust never builds these notes, so
CompactDef can't be added at build time — it is added *after* the note exists:

1. The proxy forwards the `addNote` byte-for-byte (its existing contract — it
   never alters the forwarded request).
2. In the background, once Anki has assigned a note id, it starts both of these
   at once (`tokio::join!`) and writes CompactDef when both have finished:
   - generates CompactDef from the note's word + sentence and writes it with
     `updateNoteFields`;
   - fires `vn-capture.sh` to attach audio + picture (best-effort — a stale ring
     buffer or missing audio just skips media; CompactDef still lands).

   They overlap rather than run in sequence because each is several seconds and
   each was, in turn, the thing making the other late. The capture is the one
   that cannot be delayed at all: its screenshot shows the screen as it is when
   taken, so an LLM call in front of it puts the *next* line's screen on the
   card. (Its audio window is anchored separately, at the moment the `addNote`
   arrived — `VN_ANCHOR_TS`, with `VN_NOTE_ID` naming the note — so that half is
   immune to how long anything takes.)

3. The CompactDef write is read back before it is called done
   (`anki::update_note_field_verified`). Anki answering `error: null` means it
   accepted the write, not that the value survives: a note open in Anki's
   editor gets its in-memory copy saved back over anything AnkiConnect changed
   meanwhile, silently. Verifying turns that into a logged failure naming the
   note. There is no retry — the editor is still open a second later — so the
   remedy is to leave a freshly mined card alone for a few seconds, or reopen it
   after the definition lands.

This is why CompactDef is owned by the proxy and **not** by `vn-capture.sh`:
capture aborts early exactly in the no-audio case, but CompactDef must always be
written. It also folds the old "add card, then press the mine button" into one
action.

### yt-mine (TODO — not yet built, confirmed 2026-07-27)

Still untouched: `compact_def` appears nowhere in `jp-mine-core` or `yt-mine`,
and the only `anki_compact_def_field` config lives in read-stats. The manga-mine
export path (`routes/api.rs`) shares `jp-mine-core`, so it inherits whatever is
added here — worth deciding at the time whether a manga card wants the gloss too
(ADR-005 rules out audio on those cards, not this).

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

1. **Too much to read for the word in context** — raw character count is a weak
   proxy; what matters is *how much of the sentence you must parse to pin the
   word's sense in this context*, which depends on **where the target sits**:
   - **Word at/near the start** — in review you read only up to (and just past)
     the target and stop; a long tail after it is irrelevant. e.g.
     いつも優しく朗らかで、… — you read 「いつも優しく朗らかで」 for 朗らか and never
     the rest. Do **not** flag these on length alone.
   - **Word in the middle/late, or entangled with the sentence** — you must parse
     the whole thing to get the sense, so overall length genuinely bites. Flag
     when the sentence is long *and* the word depends on a wide span around it.
   (Deck/reading reference: natural single sentences run p50 ~14, p90 ~29 chars;
   a full read-stats line averages ~3 sentences, so a card sentence much longer
   than one natural sentence usually means more context than the word needs.)
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

- [x] Design + finalized **two-axis** prompt (this doc). Signal investigation
      done — no frequency/dictionary input helps (see *Why no external signals*).
- [x] `read-stats/src/services/compactdef.rs` — LLM call (opus-5, no thinking) +
      `clean_gloss` postclean (strips echoed `<meaning>`/`<usage>` tags); live
      path validated end-to-end.
- [x] `/anki-proxy` enrichment: CompactDef write + auto vn-capture on `addNote`
- [x] Add `CompactDef` field to the note type + back template (via
      `scripts/add-compactdef-field.sh`; field at index 2, renders atop the
      Recognition card back). **Full sync still pending** — do it after backfill.
- [ ] yt-mine CompactDef wiring (TODO above)
- [x] Backfill — complete on opus-5: 681 old single-register cards re-tagged and
      1197 empty cards fully glossed (1955 two-axis total). 13 empty-sentence notes
      and 20 "new words in order" left untouched by design. **Anki full-sync still
      pending — the user's to run.**
- [–] Poor-quality-card triage (Skill B) — investigated on the live collection
      and found ~zero cull yield: forgotten cards are good sentences with weak
      backsides (the backfill's job), not bad illustrations. Bad-card removal
      stays a review-time one-keypress habit, not a batch pass.
