# Knowledge DB & module architecture

> **Status: current architecture (2026-08-01).** The other files in `spec/` are
> superseded pre-implementation design; this one describes how the code is
> actually organised.
>
> Everything below is built. Still missing: i+1 marking.

## The two-axis model

The tools looked tangled because two concerns were conflated. They are
separate axes, and a tool's place on one does not determine its place on the
other:

1. **Card authoring** — who builds the Anki card.
2. **Knowledge tracking** — what I know / what I have consumed.

### Axis 1 — card authoring

Which paradigm applies is forced by **whether a live browser dictionary
(Yomitan) sits over the content:**

| Source | Card author | Uses `jp-mine-core`? | Media attached |
|---|---|---|---|
| yt-mine (YouTube) | the app | yes | at creation |
| manga-mine (OCR crop) | the app | yes | at creation |
| VN reading (vn-mine) | **Yomitan** | no | retroactively (audio clip + screenshot from the VN process) |

A YouTube transcript or an OCR crop has no texthooker to hover words in, so
the app does the lookup, note-building and export — that is `jp-mine-core`. A VN
has one, so Yomitan authors the card and vn-mine only attaches the media Yomitan
can't reach. **Don't try to unify these.** yt/manga cannot route through Yomitan
(no popup over that content), and routing VN through `jp-mine-core` would throw
away Yomitan's popup.

### Axis 2 — knowledge tracking

Whoever made the card, every lookup / encounter / mined word is a fact about a
**term**: "word X, from source Y, at time T." That ledger is the convergence
layer every front end reads. It belongs to no single app.

## Term identity is dictionary-gated

The ledger is keyed on a canonical **`(headword, reading)`** pair, not on
per-dictionary entries and not on raw tokens. Establishing that identity
requires the dictionary layer, which is why dictionaries and the knowledge
ledger are **one subsystem** (owned by `jp-core`), not separable data.

The reading is in the key because a homograph is two words: 空 is そら or から,
辛い is からい or つらい, and marking one known must not mark the other. Over
the reading history so far that is 667 headwords carrying more than one
reading — not an edge case. `Term::new` is the only way to build the key, and
it applies two normalizations without which two writers would disagree about
which row they mean:

- **Readings fold to hiragana** (`jp_core::text::kana`). Sudachi emits katakana
  (ヨム), every Yomitan dictionary holds hiragana (よむ), and the ledger joins
  them.
- **A kana-only headword stores an empty reading.** There the two strings are
  the same fact; storing both would make ください depend on which writer got
  there first.

Three jobs all need the dictionaries:

- **Wordhood gate** — "is this incoming token even a word?" Filters reading
  noise (っ, あああ, route-finding fragments) so `#read` highlighting doesn't
  surface garbage as "unknown words." *Exists today* as `in_dictionary` in
  yt-mine (`state.dictionary_forms.contains(&lemma)`), built from
  `jp_core::knowledge::dictionaries::get_all_dictionary_forms` (all terms + readings across loaded dicts). Not yet
  wired into read-stats.
- **Canonical normalization** — map a surface token to its `(headword, reading)`
  so counts aggregate correctly. Must be **master-relative** (see below) so
  "known" and "counted" agree.
- **Classification** — a term in a name dictionary but not the master → a name,
  not vocab. (No name dict loaded yet; the schema supports it as another
  `dictionaries` row.)

### Master dictionary

Loaded dictionaries today: Sankoku (三省堂国語辞典, 81,888 distinct terms),
Jitendex (407,868), NHK (pitch only).

Jitendex is ~5× larger and **335,540 of its terms are absent from Sankoku** —
phrasal expressions (`ああ見えても`, `ああでもないこうでもない`), compositional
compounds (`あいうえお順`), and every orthographic variant of a technical term
(`α-ヘリックス` / `α－ヘリックス`) each get their own entry. A monolingual dict
lists such phrases *under* a headword; Jitendex makes them headwords. So a
vocab-size count against Jitendex is meaningless.

**Sankoku is the master dictionary.** Its ~82k-term ceiling is a real
vocabulary scale. This gives two *different* thresholds, so the ledger stores
per-term **which dictionaries contain a term** (not a single boolean) and each
feature applies its own rule:

- **Wordhood gate** (highlighter): lenient — in any dict (or master-or-name).
- **Vocab-size denominator** (dashboard): strict — **master only**. "I know
  21,230 words" = 21,230 Sankoku terms marked known or mined.

Give each `dictionaries` row a **role** (`master` / `name` / `reference`) so
adding a dict changes classification, never the vocabulary denominator.

## Mined-state: Anki stays the source

"Is this word in Anki" is **owned by Anki**, synced into the ledger as a
snapshot (the pattern read-stats' `services/anki.rs` already uses: `notesInfo` →
`anki_notes`, replaced wholesale). The ledger *caches* mined-state for fast
highlighting; a resync fixes drift. No new write paths — yt/manga/Yomitan just
make cards, Anki holds that fact.

**Mined is a flag, not a status.** `vocabulary.mined` sits beside `status`
rather than being written into it. A freshly mined word is `mined = 1,
status = 'new'`, which is accurate — it was mined *because* it wasn't known —
and it means a resync can never demote a word the reader marked known. Each
feature picks its own rule over the pair; `VocabRow::is_known` is the default,
not the law.

Both wholesale syncs match on **headword alone**, because that is all their
sources have: Anki's VocabKanji field and Yomitan's AnkiConnect request are
dictionary forms with no reading beside them. A homograph therefore takes its
mined flag and its lookup count across all its readings. That is the honest
limit of the source rather than a guess, and it fails safe for the highlighter
(a mined word is not highlighted). The fix, if it ever matters, is a reading on
the card.

### Status is assertions only

`status` holds what the reader has said and nothing else:

| | |
|---|---|
| `new` | ingested from reading, never judged — **the default** |
| `known` | I know this word |
| `unknown` | judged, and not known — also what the triage sweep's "no" writes |
| `blacklisted` | never surface this again |

`learning` and `name` were removed in 2026-07, both at zero rows: `learning`
duplicated `mined`, and ingest drops names before they reach the ledger. Old
values read back as `new`.

`new` stays distinct from `unknown`. The two look the same to i+1 counting and
to the highlighter, but once ingest has written `unknown` across every word ever
read, no migration can reconstruct which were actually judged. That distinction
is what makes cold-start.md's Pass 4 ("seen 12 times, never judged, do you know
it?") and the progress figure answerable. It costs one allowed value in a TEXT
column.

No writer other than the reader touches `status` — not ingest, not the Anki
sync, not the lookup sync. Encountering a word again says nothing about whether
it is known, which is exactly Pass 4's caveat; auto-promotion on encounter
count is the one thing cold-start.md rules out, and the schema is arranged so
it can't happen by accident.

## Encounters are implicit — counts live on the ledger row

There is **no per-occurrence encounter table**, and `word_days` is on its way
out (see the end of this section). Both are fully derived data: the raw truth of
"every occurrence of a term" already lives in `lines` (and in `manual_sessions`
once it carries its content — see below). Storing a derived copy violates
"don't store what you can derive."

Instead the ledger row carries running aggregates — `encounter_count`,
`lookup_count`, `first_seen`, `last_seen` — written by the same ingest that
tokenizes new lines (`read-stats/src/ingest.rs`, watermarked on `settings`).
That gives the `#read` highlighter an O(1) status lookup per token, cheap
enough to run as each line arrives.

**Each sink has its own watermark.** `word_days` and the ledger are filled by
one tokenization pass but tracked separately (`tokenized_through_line_id` vs
`vocab_through_line_id`, same pair for sessions). That is what made the ledger
backfillable: it arrived 11,859 lines into a history `word_days` had already
counted, and a shared watermark would have forced a choice between an empty
ledger and double-counted days. The sinks are additive and not idempotent, so a
row is written to a sink only when its id is past *that sink's* watermark.
`POST /api/vocab/rebuild` rewinds the ledger's pair alone — also the repair path
for any re-tokenization (a Sudachi upgrade, a change to `is_content_word`).

The highlighter reads counts + status per token, no history scan. Any stat a
plain count can't answer ("mined words never re-encountered since their mined
day") is derived on demand from `lines`, which carries `ts` — cheap at this
scale and off the hot path. Time-windowed variants ("encounters this week") are
**not** a design constraint.

`word_days` exists only because there was no ledger to compute its one consumer
from — the mined-word re-encounter panel (`routes/anki.rs`,
`fetch_mined_word_days`). It can be dropped once that panel is recomputed from
`lines` + the ledger.

**Not done yet.** Table and consumer are both live
(`read-stats/src/db/word_days.rs`, `routes/anki.rs:92`) and ingest still fills
it. The panel asks "of the words I carded, which has the reading shown me
again?", which needs a trustworthy `mined` flag.

## Database layout

Three DB files, split on **reference/knowledge (shared) vs. activity-specific
event streams** — not on "which app."

### `knowledge.db` — owned by `jp-core`, shared

The dictionary cache + the knowledge ledger + the raw streams and source
dimension that feed it. Everything here is dictionary-gated or joins the ledger.

- `dictionaries` (+ **role** column), `dictionary_entries`, `dictionary_pitch`,
  `dictionary_frequency` — *moved here 2026-07*.
- `vocabulary` — the ledger, one row per `(headword, reading)`: status, mined
  flag, aggregate counts, dictionary flags. *Built 2026-07-26.* The empty stub
  that sat in `yt-mine.db` is superseded, not moved: it was keyed on lemma
  alone and carried a `user_id` this workspace has no use for. yt-mine's
  `routes/vocab` still writes that stub and is the next consumer to migrate.
- `anki_notes` — mined-deck snapshot mirror. *Moved here 2026-07.*
- `word_days` — per-day content-word counts from the line stream. *Moved here 2026-07.*
- `works` — the **source dimension** of encounters (a VN/video/book is a
  source). Joined by `lines.work` / `manual_sessions.work`. Kotodex's encounter
  map aggregates by it. Carries display fields (cover, status, queue_pos) too,
  but its identity is the knowledge layer. *Moved here 2026-07.*
- `lines` — raw hooked VN lines; tokenized into the ledger's counts and joined
  against the dict for `#read` highlighting. *Moved here 2026-07.*
- `manual_sessions` — manually entered reading time (renamed from `sessions`).
  Carries `content TEXT` — the actual text read (online article, ebook,
  YouTube transcript, a physically-read book typed/pasted later) — and a `url`
  beside it. When content is present it *is* the character count, via the same
  `jp_core::text::chars::count_chars` the line stream uses, so a pasted
  article's speed is comparable with a VN's instead of being pages × a
  constant. The content lives on the session row itself — it is **not**
  expanded into `lines` rows. *Moved here 2026-07 and renamed; `content` +
  `url` landed 2026-07-26.*

  `end_ts` is nullable: reading off paper is not timed, and an untimed
  session's duration is derived from the reader's own effective pace rather
  than stored (`History::duration_of`). *2026-07-26.*

  `content` is tokenized into both sinks by `ingest::ingest_new_sessions`,
  behind session watermarks of its own (`tokenized_through_session_id` /
  `vocab_through_session_id`), so manual and live reading feed the same
  knowledge state. It landed after the split that made it safe: article lookups
  are never captured, so article characters feed every **exposure** count and no
  **cost** count — `stats/kanji.rs` carries `metered_count` beside `count` for
  that. *Done 2026-07-26.*

**read-stats writes into the shared DB** (line ingestion, the highlighter's
status reads). It is not a pure reader.

### `read-stats.db` — read-stats internal

Only tables that never join the knowledge layer:

- ~~`pauses`~~ — retired. Pausing stops capture at the logger now, so there
  is no interval to filter and no table to keep.
- `settings` — `current_work`, ingest watermarks, app state.
- `reader_marks` — presence/AFK proof only; deliberately kept out of word
  metrics so it can't inflate lookup counts.

### The AnkiConnect proxy is split across layers

Yomitan points its "server address" at read-stats' proxy endpoint
(`routes/ankiproxy.rs`), which forwards byte-for-byte to real AnkiConnect and records
each lookup. This is two concerns with two homes:

- **The lookup write** (`insert_lookup` → the `lookups` ledger) is core knowledge
  data — it moves to **`jp-core`**'s db layer with the rest of the ledger.
- **The proxy HTTP handler** stays in **read-stats**. `jp-core` is a pure library
  (no Axum/server), so an endpoint can't live there; and Yomitan points at one
  always-on address, which read-stats (the always-on reading hub) is the natural
  host for.

### `yt-mine.db` — yt-mine internal

- `mining_jobs`, `mining_sentences` — transcription cache. The only genuinely
  YouTube-specific tables; everything else the old `yt-mine.db` held was shared
  dictionary/ledger data that moves to `knowledge.db`.

## Module summary

- **`jp-core`** — language primitives: tokenize, dictionary, and the
  `knowledge.db` layer (dictionaries + ledger). The knowledge subsystem lives
  here because it *is* dictionary-gated.
- **`jp-mine-core`** — card-authoring back half: note builder (Sankoku full +
  Jitendex collapsed) + AnkiConnect export. Used by yt-mine and manga-mine;
  correctly unused by read-stats/vn-mine (Yomitan authors there).
- **front ends** — yt-mine, manga-mine, read-stats, future kotodex: compose the
  above; own their activity-specific event streams.

## Migration notes

Done (2026-07-25):

1. ✅ `dictionaries` / `dictionary_*` moved out of `yt-mine.db`, and `works` /
   `lines` / `sessions` / `word_days` / `anki_notes` / `lookups` out of
   `stats.db`, into `knowledge.db`, by a one-time script since deleted.
   `vocabulary` was left in `yt-mine.db` at the time; that stub has since been
   deleted (note 8).
2. ✅ `dictionaries.role` (`master` / `name` / `reference`) exists;
   `jp_core::knowledge::dictionaries::ensure_master` marks Sankoku at startup
   from `JP_TOOLS_MASTER_DICTIONARY`. Nothing reads the role yet — it is
   schema and plumbing, waiting for the denominator that needs it.
3. ✅ `sessions` → `manual_sessions`.

Done (2026-07-26):

4. ✅ The `vocabulary` ledger exists and is populated
   (`jp_core::knowledge::vocabulary`, migration `005`). Ingest is
   reading-aware; the three wholesale syncs (mined, lookup counts, dictionary
   flags) run after every Anki refresh; `POST /api/vocab/rebuild` backfills
   from the whole history. First backfill, 2026-07-26: 7,949 terms from 10,649
   lines + 1 session, 6,347 of them master-dictionary vocabulary.

   Ten unit tests in `vocabulary.rs` pin the invariants this document argues
   for, and they are the regression net for anything below: homographs stay
   separate terms, a reading folds to hiragana, a kana headword stores no
   reading, an assertion survives re-ingest, the Anki sync sets `mined`
   without touching `status`, lookup counts are recomputed rather than
   accumulated, dictionary flags follow the role and not the dictionary, and a
   status can be asserted before the word is ever read.

   **Every count in this file and in `spec/cold-start.md` is from the master
   database, which lives on one machine.** A dev checkout's
   `~/.local/share/jp-tools/knowledge.db` is an older snapshot kept for
   development, and tables can legitimately be empty there — an empty
   `vocabulary` plus a missing `vocab_through_line_id` means the backfill has not
   been run *on that copy* (fix: `POST /api/vocab/rebuild`), and an empty
   `dictionary_frequency` or `dictionary_entries` means the dictionaries have not
   been imported there. Neither is a regression. Verify behaviour against the
   tests, not against a snapshot's row counts.

Done (2026-07-27):

5. ✅ **Triage** — the first writer of `status`, and read-stats' `#vocab` tab.
   `spec/cold-start.md`'s Pass 2 over terms already in the ledger:
   `GET /api/vocab/queue` offers untriaged master-dictionary terms above an
   encounter floor, most-met first; `POST /api/vocab/judge` writes a mixed batch
   of `known`/`unknown` in one transaction; `POST /api/vocab/blacklist-non-words`
   clears the tail nothing recognises as a word. The tab also shows the ledger's
   status counts, which is the progress figure the pass moves.

   **The preselect rule is the load-bearing part**
   (`vocabulary::preselects_known`): a word is ticked `known` only if it was met
   at least `triage_min_encounters` times **and was never looked up**. Encounters
   alone cannot tell "read straight past it" from "looked it up on twelve of
   those times", and the second is the profile of a word the reader does not
   have. Deliberately the same predicate as Pass 4's review query, so the
   ongoing pass is a re-run of this one rather than a second rule that can drift
   from it.

   The floor is `settings.triage_min_encounters`, default **3** — low because
   the lookup half of the rule carries the weight — and previewable per request
   (`?min_encounters=`) so the UI can show what moving it does before saving.

   Judging is confined to the batch on screen: a submit writes verdicts for
   those rows and no others, so an interrupted pass leaves a resumable queue
   rather than a ledger of guesses. `POST /api/vocab/judge` rejects an
   unrecognised status rather than falling back to `Status::parse`'s `new`,
   which would silently un-judge a row while reporting success.

7. ✅ **The `#read` highlighter** — `read-stats/src/routes/reader/highlight.rs`.
   The ledger's first reader. Ingest's Sudachi pipeline runs over each streamed
   line; each content word is returned as an *offset*, not markup, so the line
   stays one text node and Yomitan's DOM scan is unaffected.

6. ✅ **Anki import (Pass 1)** — `POST /api/vocab/anki-import`. Its own pass and
   not a line in the mined sync, because the sync only flags rows that exist:
   at the first backfill 431 of 1,995 deck words matched. The rest are
   multi-word expressions Sudachi never emits whole (腹を探る, 相好を崩す) and
   words mined
   from yt/manga rather than read.

   Gated on Anki's queue — `findNotes "deck:X -is:new -is:learn"` — and
   imported as `known`; a card in active review is ~90% reliable evidence and
   the vocabulary count is an estimate anyway. A card still in the new/learning
   queue is a word explicitly not yet had, so those notes are left alone.

   Reader-triggered, never part of the recurring refresh. Only the reader
   writes `status`; an import is them saying "trust my deck" once.

9. ✅ **jiten.moe seed (Pass 5)** — `POST /api/vocab/jiten-import`. Cards carry
   JMdict `ent_seq`, so nothing has to be inferred or skipped as ambiguous.
   jiten's maturity grades are ignored: every card imports as `known`.

Done (2026-08-01):

8. ✅ yt-mine's own lemma-keyed `vocabulary` table and its `/vocab` calibration
   page were **removed** rather than migrated. Both local copies held 0 rows,
   and its `seen`/`known`/`blacklisted` status vocabulary was incompatible with
   this ledger's. read-stats' `#vocab` tab is the triage UI now. If yt-mine
   ever wants "how many unknown words are in this video", it reads
   `jp_core::knowledge::vocabulary` directly.

### Where the queries live

The schema for the shared tables is jp-core's — it has to have one owner — but
the *query helpers* for the reading tables are still in read-stats (`db/`),
because read-stats is still their only caller. Moving them up before a second
consumer exists would be inventing an interface with nothing to test it
against. When kotodex or the highlighter needs them, they move; the tables
won't have to.
