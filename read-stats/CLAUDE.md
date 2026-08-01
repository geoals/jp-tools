# read-stats — daily reading tracker + the `#read` reading view

Rust 2024. Axum JSON API + Preact/htm frontend (no build step), two SQLite
databases. Port 3200.

- **the dashboard** — how much was read, how fast, how continuously, what it
  cost in lookups. Everything is derived from the raw line stream at query time,
  so changing a threshold re-reads the whole history under the new rule.
- **`#read`** — the live line feed read beside the running VN, the explain
  button, and the AnkiConnect proxy Yomitan points at.

## The shape of the thing

```
                  vn-ws-logger.py                     Yomitan
                        │ appends                        │ AnkiConnect
                        ▼                                ▼
                 knowledge.db: lines            routes/ankiproxy.rs
                        │                                │ records
                        ▼                                ▼
   history.rs  ◄─── one load per request ───►   knowledge.db: lookups
        │
        ▼
    stats/ ── pure derivation ──►  routes/ ──► JSON ──► static/
```

Nothing in `stats/` touches a database, a clock or a timezone; every threshold
arrives as a parameter. That is what lets `tests/api.rs` assert exact numbers.

`src/lib.rs` is the layer map. Read `stats/presence.rs` first — how much of a
gap counts as reading is the decision everything else builds on. `clock.rs`
holds the only impure inputs; each `db/` module doc says which database it
talks to.

## Two databases

`knowledge.db` is shared and its schema is owned by `jp_core::knowledge`
(`lines`, `works`, `manual_sessions`, `anki_notes`, `word_days`, `lookups`,
`vocabulary`, the dictionary cache). `read-stats.db` is this app's own:
`settings`, `reader_marks`, `work_covers`. `db` functions take a `Knowledge`
handle or a bare `SqlitePool`, so passing the wrong database is a compile error.
The two places that straddle the line — the current work's capture window and
the cover sources — join in memory; keep it that way.

## Invariants

Measurement:

- **Presence is the rule everything credits time through.** A new aggregate
  that measures time goes through `stats::Presence`, not a fresh
  `min(gap, cap)`. When those diverged, the focus metric punished the reader for
  using a dictionary.
- **Pace is a property of the reader, not of a request.** `History` derives it
  once over all history, or the dashboard and the day timeline disagree about
  the same day.
- **Speed divides by measured reading only** (`History::measured_days`). An
  untimed session's duration is derived from the reader's own pace, so in a
  speed chart it would measure its own output. Totals, goals and streaks still
  count everything read.
- **Exposure counts take all text; cost counts take only hooked text.** Pasted
  session `content` feeds `word_days`, the kanji grid and every coverage figure,
  but stays out of every rate — `lookups_per_1k` divides by hooked characters.
- **`chars` excludes punctuation** (`jp_core::text::chars`), matched to
  texthooker-ui so speeds are comparable with other people's. Startup recounts
  the column.

The line stream:

- **Nothing is deleted.** A line that shouldn't count gets `discarded = 1`,
  filtered on read.
- **Pausing stops capture, it does not filter.** vn-ws-logger.py polls
  `settings.capture_paused` and closes its Textractor WebSocket while it is set,
  so a paused span simply has no lines in it.
- **A lookup only exists if it happened while reading.** Yomitan fires the proxy
  for anything looked up anywhere, so `ankiproxy::record` records only when a
  line arrived within `session_gap_secs`. The guard is at the write and nowhere
  else — don't add a second filter downstream.

The ledger (`vocabulary`):

- **Only the reader writes `status`.** Not ingest, not the Anki sync, not the
  lookup sync — a resync must never demote a word marked known, and an encounter
  count must never promote one (`spec/cold-start.md` Pass 4). Today's writers:
  `/api/vocab/judge`, `/api/vocab/blacklist-non-words`, the tap in `#read`, and
  the `anki-import` / `jiten-import` / frequency imports.
- **`new` ≠ `unknown`.** `new` means never judged; collapsing them is
  irreversible and breaks the triage progress figure.
- **Anki owns mined-state.** `anki_notes` is a snapshot, replaced wholesale,
  never written back. `vocabulary.mined` is recomputed from it and is a flag
  beside `status`, never written into it.
- **A word judged under one reading is not asked about again**, and not marked
  under another either. 皆 marked known as みな means 皆/みんな is never offered.
- **Each ingest sink has its own watermark.** One pass fills `word_days`, the
  ledger and `work_terms`, but their three watermarks move independently. The
  sinks are additive and not idempotent, so a row goes to a sink only when its
  id is past *that sink's* mark — which is what lets `POST /api/vocab/rebuild`
  re-derive the ledger without double-counting.

Tokenization (all in `jp_core::tokenize`, shared with the highlighter so a tint
and a ledger row cannot disagree):

- **A term's reading is the reading of its headword**, not of the surface —
  otherwise 知る splits across しる, しら and しっ.
- **One word, one row, spelt the way the master dictionary spells it.** Terms
  key on Sudachi's *normalized* form. Where Sudachi and Sankoku disagree,
  Sankoku wins (`written_form`).
- **Compounds the master dictionary doesn't list are decomposed into parts it
  does** (`decompose`), and **adjacent parts it lists as one word are rejoined**
  (`recompose`). Names are never decomposed and never rejoined; bare kana is
  excluded from decomposition.
- **A name is not vocabulary** — 固有名詞 keeps a work's cast out of the ledger.
  The verdict is per *term* over a whole pass, never per occurrence.
- **An affix the master dictionary lists is a word.** `counts_as_word` admits
  接尾辞/接頭辞 when the master lists the `(headword, reading)` pair — that test
  is the whole fence, with no stoplist to maintain.
- **A re-tokenization strands judgements, and the rebuild re-homes them.**
  `carry_stranded_judgements` moves a status to whatever the term is called now,
  never over the target's own assertion.

The reading view:

- **A tap in the feed judges the word under it.** Two states: anything marked
  becomes `known`, a word already known becomes `unknown`. `new` and `seen` are
  unreachable by hand and must stay that way. No undo and no toast — the mark is
  the report, and a failed write is the mark coming back. It is hit-tested with
  `caretPositionFromPoint`, and **nothing in the feed is made clickable**: an
  interactive layer would sit between the reader and the text Yomitan scans.
- **Marks are drawn, never markup.** `routes/reader/highlight.rs` sends offsets
  per line and `paintMarks` draws a rectangle per word into a layer *behind* the
  text. Yomitan scans this DOM, so one text node per line is a constraint.
  Offsets are UTF-16 code units because that is what a `Range` indexes in.
  Three tiers are painted and `known` is not one of them — the absence of a mark
  is what makes the marks readable — but a `known` span is still sent, since a
  span is also the region a tap judges.
- **The feed re-pins to the bottom on a new *line*, not on a new `lines`** —
  judging a word rebuilds the array without adding to it, and an id-keyed pin
  kept yanking the word out from under the finger. A reflow re-pins too, on a
  height test (`pinToBottom`), because the web font, a page of history and a
  resize all move the feed under a reader who never touched it.
- **The `◌ marked` filter is a view, never a write.** It filters on membership
  (`keptIds`), not a live predicate, or judging the last marked word in a line
  deletes that line from under the finger. `lines` stays the whole feed and the
  filter applies at the last moment, so everything that measures or hit-tests
  text takes `visible` — and the repaint must depend on `keptIds`, not only on
  `lines`, or a backscroll strands every mark a page-height off its word.

Mining:

- **Mining is implicit.** Yomitan's `addNote` goes through `routes/ankiproxy`,
  which fires vn-capture.sh once Anki accepts the note. There is no mine button.
- **A capture is anchored at the add, not at the capture.** The proxy stamps
  `now_ts()` when `addNote` arrives and passes it as `VN_ANCHOR_TS`. Nothing may
  be awaited in front of the capture: in `enrich_added_note` the CompactDef call
  runs *alongside* it (`tokio::join!`) with its Anki write after. The two
  `updateNoteFields` stay strictly ordered.
- **An accepted Anki write is not a stored value.** If the note is open in
  Anki's editor, the editor's next save overwrites the field with nothing
  logged. The CompactDef path uses `anki::update_note_field_verified`, which
  reads the field back. It does not retry — don't open a freshly mined card for
  a few seconds.
- **The chime is the only report a mine gets.** `services::chime::mine_complete`
  plays only when the capture reported `ok` *and* the CompactDef write verified.
  Keep it that strict: silence is the signal to check the log.
- **The audio window's next-line bound is a hard cut, and that is a known
  defect.** When the next line is unvoiced the previous voice legitimately
  plays past its timestamp and the clip is truncated. It shipped that way
  because a truncated clip of the right line beats a whole clip of the wrong
  one; `vn-mine/vn-calibrate.py` is the tool that would replace the rule, and
  it needs a real session's data first.
- **Note ids are epoch milliseconds**, so they double as card creation times.
- **Only engagement actions leave `reader_marks`.** Explain does; clear does not.

## Not built

Per-work **difficulty**, two measures side by side: *text difficulty* (share of
tokens outside the frequency core, share of non-jōyō kanji, mean sentence
length) which stays put as the reader improves, against *measured cost*
(lookups/1k and chars/hour vs baseline) which moves. Plotting every work one
against the other puts engagement in the residual — a work read faster than its
prose predicts. Neither figure exists for a work with no reading behind it.

Also unbuilt: i+1 marking, and a "read often, never carded" list on `#kanji`.

## Working on it

Don't restart the live stack or touch `~/.local/share/jp-tools` while a VN is
being read. Use an isolated instance:

```sh
scripts/dev-instance.sh run             # :3299, on a frozen copy of the data
scripts/dev-instance.sh snapshot before # record every endpoint
# ...make the change...
scripts/dev-instance.sh check before    # must print IDENTICAL
scripts/dev-instance.sh browser         # the SPA actually renders
```

For a refactor that must not change behaviour, the snapshot diff is the proof.
The browser check exists because the client is unbundled ES modules loaded
straight from disk: a bad import path renders *nothing at all* while every JSON
endpoint still passes.

`run` holds the terminal and has no `stop`, so a backgrounded instance outlives
the session. Take a free port (`DEV_PORT=3298`) rather than clearing it.
**Never `pkill -f` your way out of that** — the dev instance and the live :3200
service are the same binary path. Resolve the PID from the port instead
(`ss -ltnp | grep :3299`).

```sh
cargo test -p read-stats     # unit + integration (tests/api.rs)
```

`tests/api.rs` runs the real router against a throwaway database — the layer to
add to when the question is "does the SQL select what the derivation assumes".

## Frontend notes

- Preact + htm from a CDN import map, no build step. `charts.js` and
  `style.css` are re-export/`@import` facades — add a chart or a sheet there.
- **Never let literal text and `${...}` straddle a line break inside an `html`
  template.** htm collapses the whitespace, which silently rendered
  `snapshot 0 min ago` as `snapshot0 minago`. Build the string in JS and
  interpolate it whole.
- The dashboard polls once and passes the result down — half the cards are
  different readings of the same days. **Tabs choose what renders, never what is
  fetched.**
- Five tabs, one per question: **Today** (`current-reading.js` over `day.js`),
  **Trends** (one range selector over every chart), **Library**, **Kanji**,
  **Vocab**. Plus `#read` and `#tokenize`, reached from the header.
- **Library has two levels.** The shelf lists works as cards; opening one
  replaces the tab with `work-detail.js` over `GET /api/works/detail`, keyed by
  title. A work with no reading behind it does not appear. Logged articles
  collapse into one `Articles` row (`stats::work::ARTICLES_WORK`). The log form
  has two modes over one POST: *pages* estimates chars from a page count,
  *paste text* counts the article exactly, via `/api/text/count` rather than a
  `length` in JS.
- **`#tokenize` reports the tokenizer, not the ledger's folding.**
  `Analyzed.reading` is the reading the token was produced with; where the
  status came from a different row, `judged_as` carries that row's reading in
  its own column. The feed folds them in `spans`, on the way out of `analyze`.
  The page writes nothing — no ledger row, no count, no presence mark.
- **A bulk write shows its rows first.** `blacklist-non-words` judges rows the
  queue never displays, so `GET /api/vocab/non-words` lists them and the button
  only appears once they are on screen.
- **Triage ticks on two signals, never one** (`vocabulary::preselects_known`): a
  word is preselected `known` only if it was met at least
  `triage_min_encounters` times **and was never looked up**. Unticked means
  `unknown` on submit, so a one-signal default would write wrong assertions in
  bulk. The rule lives server-side because it decides what gets written.
- **The sweep is scoped to what has been read since the last one.**
  `sweep_through_ts` is compared against `vocabulary.last_seen`. It moves
  **on submit, never on load**; only for a
  request that asked (`advance_sweep`); and it is a filter and nothing else —
  `scoped=0` still reaches every ready row.
- **Status colour is one scale, in HSL, in `base.css`.** Hue names the status
  (211 blue `new` / 276 violet `seen` / 28 amber `unknown`), lightness says how
  loudly, and the dark ramp mirrors the light one. Both places that show a
  status read these, so the tint under a word in the feed is the colour of the
  pile it is counted in.
- Selected state has one vocabulary: `background: var(--meter-track)` with
  primary ink (`.segment-on`, `.toggle-on`, `.tab-on`). `--series-1` at full
  strength is spent on the paused alarm alone.
