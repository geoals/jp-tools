# Knowledge DB & module architecture

> **Status: current, decided direction (2026-07).** Unlike the other files in
> `spec/` (which are the superseded pre-implementation design), this reflects
> the architecture the codebase is actually moving toward. It records decisions
> reached about how the tools share data. Not all of it is built yet — sections
> mark what exists today vs. what is planned.

## The two-axis model

The tools looked tangled ("is this three separate apps or one workspace?")
because two independent concerns were conflated. They are separate axes:

1. **Card authoring** — *who builds the Anki card.*
2. **Knowledge tracking** — *what I know / what I have consumed.*

A tool's place on axis 1 does not determine its place on axis 2. Keeping them
separate is what makes the rest of the design fall out cleanly.

### Axis 1 — card authoring

Which paradigm applies is forced by **whether a live browser dictionary
(Yomitan) sits over the content:**

| Source | Card author | Uses `jp-mine-core`? | Media attached |
|---|---|---|---|
| yt-mine (YouTube) | the app | yes | at creation |
| manga-mine (OCR crop) | the app | yes | at creation |
| VN reading (vn-mine) | **Yomitan** | no | retroactively (audio clip + screenshot from the VN process) |

For a YouTube transcript or an OCR crop there is no texthooker to hover words
in, so the app must do the lookup + note-building + export — that is exactly
what `jp-mine-core` is. For a VN there *is* a texthooker, so Yomitan (with its
dictionaries, pitch, deck config) authors the card; vn-mine only attaches the
media Yomitan can't reach. **This split is principled — do not try to unify it.**
Routing yt/manga "through Yomitan" is not achievable (no popup over that
content); routing VN through `jp-mine-core` would throw away Yomitan's popup.

### Axis 2 — knowledge tracking

Independent of who made the card, every lookup / encounter / mined word is a
fact about a **term**: "word X, from source Y, at time T." That ledger is the
convergence layer every front end reads and future tools (kotodex, `#read`
highlighting) build on. It does **not** belong to any one app — today it is
scattered and partly missing.

## Term identity is dictionary-gated

The ledger is keyed on a canonical **`(headword, reading)`** pair, not on
per-dictionary entries and not on raw tokens. Establishing that identity
requires the dictionary layer, which is why dictionaries and the knowledge
ledger are **one subsystem** (owned by `jp-core`), not separable data.

Three jobs all need the dictionaries:

- **Wordhood gate** — "is this incoming token even a word?" Filters reading
  noise (っ, あああ, route-finding fragments) so `#read` highlighting doesn't
  surface garbage as "unknown words." *Exists today* as `in_dictionary` in
  yt-mine (`state.dictionary_forms.contains(&lemma)`), built from
  `get_all_dictionary_forms` (all terms + readings across loaded dicts). Not yet
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
compounds (`あいうえお順`), and every orthographic variant of technical terms
(`α-ヘリックス` / `α－ヘリックス`) each get their own entry. A monolingual dict
lists such phrases *under* a headword; Jitendex makes them headwords too. So a
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
snapshot (the pattern read-stats' `anki.rs` already uses: `notesInfo` →
`anki_notes`, replaced wholesale). The ledger *caches* mined-state for fast
highlighting; a resync fixes drift. No new write paths — yt/manga/Yomitan just
make cards, Anki holds that fact.

The ledger owns only what Anki can't tell you: lookup counts, encounter counts,
and derived status (`unseen` / `seen` / `learning` / `known` / `blacklisted` /
`name`).

## Encounters are implicit — counts live on the ledger row

There is **no separate per-occurrence encounter table**, and **no `word_days`
table**. Both would be fully derived data: the raw truth of "every occurrence of
a term" already lives in `lines` (and in `manual_sessions` once it carries its
content — see below). Storing a derived copy violates "don't store what you can
derive."

Instead, the `vocabulary` ledger row carries running aggregates —
`encounter_count`, `lookup_count`, `last_seen_ts` — incremented by the same
incremental ingest that tokenizes new lines today (`read-stats/src/ingest.rs`,
watermarked on `settings`). This is what the planned `#read` highlighter needs:
an O(1) per-token status lookup, viable to run as each line arrives.

The highlighter reads the ledger's aggregate counts + status per token — no
history scan. Any dashboard stat that a plain count can't answer (e.g. "mined
words never re-encountered since their mined day") is derived on demand from
`lines`, which carries `ts`; cheap at this scale and off the hot path.
Time-windowed variants (e.g. "encounters this week") are **not needed** and are
not a design constraint.

`word_days` exists today only because there was no ledger to compute its one
consumer from — the mined-word re-encounter panel (`api.rs`, `fetch_mined_word_days`).
That panel is recomputed from `lines` + the ledger; `word_days` is dropped.

## Database layout

Three DB files, split on **reference/knowledge (shared) vs. activity-specific
event streams** — not on "which app."

### `knowledge.db` — owned by `jp-core`, shared

The dictionary cache + the knowledge ledger + the raw streams and source
dimension that feed it. Everything here is dictionary-gated or joins the ledger.

- `dictionaries` (+ **role** column), `dictionary_entries`, `dictionary_pitch`,
  `dictionary_frequency` — *exist today, currently misfiled in `yt-mine.db`*.
- `vocabulary` — the ledger, one row per `(headword, reading)`: status, mined
  flag, aggregate counts. *Table exists in `yt-mine.db` but is empty — the false
  start this design fills.*
- `encounters` — append-only, per-term, tagged `source_type` + `source_title` +
  `ts`. Generalizes today's VN-only `lookups`. *Planned.*
- `anki_notes` — mined-deck snapshot mirror. *Exists in `stats.db` today.*
- `word_days` — per-day content-word counts from the line stream. *Exists in
  `stats.db`.*
- `works` — the **source dimension** of encounters (a VN/video/book is a
  source). Joined by `lines.work` / `manual_sessions.work`. Kotodex's encounter
  map aggregates by it. Carries display fields (cover, status, queue_pos) too,
  but its identity is the knowledge layer. *Exists in `stats.db`.*
- `lines` — raw hooked VN lines; tokenized into the ledger's counts and joined
  against the dict for the planned `#read` highlighting. *Exists in `stats.db`.*
- `manual_sessions` — manually entered reading time (renamed from `sessions`).
  Gains a `content TEXT` column holding the actual text read (online article,
  ebook, YouTube transcript, a physically-read book typed/pasted later). Import
  = tokenize that `content` into the ledger's counts, so manual and live reading
  feed the same knowledge state. The content lives on the session row itself —
  it is **not** expanded into `lines` rows. *Exists in `stats.db` as `sessions`;
  `content` is new.*

Consequence: **read-stats writes into the shared DB** (line ingestion, the
highlighter's status reads). It is not a pure reader — stated so ownership is
honest.

### `read-stats.db` (or keep `stats.db`) — read-stats internal

Only tables that never join the knowledge layer:

- `pauses` — derivation input: which lines count. Never joined to knowledge.
- `settings` — `current_work`, ingest watermarks, app state.
- `reader_marks` — presence/AFK proof only; deliberately kept out of word
  metrics so it can't inflate lookup counts.

### The AnkiConnect proxy is split across layers

Yomitan points its "server address" at read-stats' proxy endpoint
(`ankiproxy.rs`), which forwards byte-for-byte to real AnkiConnect and records
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

## Migration notes (not yet done)

1. Move `dictionaries` / `dictionary_*` and the `vocabulary` stub out of
   `yt-mine.db` into `knowledge.db`; move `works` / `lines` / `sessions` /
   `word_days` / `anki_notes` out of `stats.db` into `knowledge.db`.
2. Add `dictionaries.role` (`master` / `name` / `reference`); mark Sankoku
   master.
3. Rename `sessions` → `manual_sessions`.
4. Build the `encounters` log; generalize `lookups`' source tag; populate
   `vocabulary`.
5. Wire the wordhood gate + status lookup into read-stats' `#read` highlighter.
