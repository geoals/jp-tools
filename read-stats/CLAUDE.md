# read-stats — daily reading tracker + the `#read` reading view

Rust 2024. Axum JSON API + Preact/htm frontend (no build step), two SQLite
databases. Port 3200.

Two things live here:

- **the dashboard** — how much was read, how fast, how continuously, and what
  it cost in lookups. Everything on it is *derived* from the raw line stream at
  query time, so a threshold can be changed and the whole history re-reads
  under the new rule.
- **`#read`** — the live line feed read beside the running VN, plus the explain
  button and the AnkiConnect proxy Yomitan points at. Usually a browser on this
  machine; it is served over the LAN (and Tailscale) too, so a second device
  beside the screen works the same way.

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

Nothing in `stats/` touches a database, a clock or a timezone: every threshold
arrives as a parameter, which is what makes the derivations unit-testable and
what lets `tests/api.rs` assert on exact numbers.

## Where to start

`src/lib.rs` is the layer map — read it first. Then `stats/presence.rs`: how
much of a gap counts as reading is the decision everything else is built on.
`clock.rs` holds the only impure inputs (`now_ts`, `tz_offset_secs`); every
`db/` module doc says which of the two databases it talks to.

## Two databases

| | |
|---|---|
| `knowledge.db` | shared, schema owned by `jp_core::knowledge`: `lines`, `works`, `manual_sessions`, `anki_notes`, `word_days`, `lookups`, `vocabulary`, and the dictionary cache |
| `read-stats.db` | this app's own: `settings`, `reader_marks`, `work_covers` |

The split is `spec/knowledge-db.md`'s: what is *about the reading* is shared,
because other tools ask questions of it; what is about this app's behaviour is
local. `db` functions take a `Knowledge` handle or a bare `SqlitePool`
accordingly, so passing the wrong database is a compile error rather than a
query against an empty table that was just created for it.

Two places straddle the line — the current work's capture window and the cover
sources — and both join in memory rather than attaching one database to the
other. Keep it that way; a cross-database join here would also have to be
taught to `vn-capture.sh`.

## Invariants worth knowing before changing anything

- **Presence is the rule everything credits time through.** If you add an
  aggregate that measures time, it goes through `stats::Presence`, not through
  a fresh `min(gap, cap)`. The last time those two diverged, the focus metric
  punished the reader for using a dictionary.
- **Pace is a property of the reader, not of a request.** `History` derives it
  once over all history. Deriving it per-endpoint is what made the dashboard
  and the day timeline disagree about the same day.
- **Nothing is deleted.** A line that shouldn't count gets `discarded = 1`,
  filtered on read — so it can be undone and the raw stream is never lost.
- **Pausing stops capture, it does not filter.** `settings.capture_paused` is
  polled by vn-ws-logger.py, which closes its Textractor WebSocket and stays
  disconnected while it is set. There is no interval log any more: a paused
  span has no lines in it, so nothing needs excluding. The old `pauses` table
  is retired on startup by `db::retire_pauses` (see its module doc for what
  happened to the rows it covered).
- **A lookup only exists if it happened while reading.** Yomitan points at the
  proxy from the *browser*, so it fires for anything looked up anywhere.
  `ankiproxy::record` records a lookup only when a line arrived within
  `session_gap_secs`, so reading a news article never puts a term in a VN's
  funnel and never inflates the day's per-1000-character rate. The guard is
  at the write and nowhere else — don't add a second filter downstream, and
  don't remove this one and expect the readers to cope. It also means a long
  enough `capture_paused` stops lookups: no lines arrive, so nothing is recent.
- **Exposure counts take all text; cost counts take only hooked text.** Pasted
  session `content` feeds `word_days`, the kanji grid, the discovery curve and
  every coverage figure — those ask how much you have met something, and an
  article is reading. It is kept out of every *rate*: `lookups_per_1k` divides
  by hooked characters (`stats::rate`). A lookup can only be recorded while the
  line stream is live, so article characters could enter a rate's denominator
  but never its numerator, and the rate would fall in proportion to how much of
  the reading is articles. If you add a figure, decide which of the two
  questions it asks before choosing its denominator.
- **Speed divides by measured reading only** (`History::measured_days`). An
  untimed session's duration is derived from the reader's own pace, so it
  reports that pace back exactly; in a speed chart it would be measuring its
  own output. Totals, goals and streaks still count everything read.
- **Anki owns mined-state.** `anki_notes` is a snapshot, replaced wholesale.
  Never write back. The same applies one layer up: `vocabulary.mined` is
  recomputed from that snapshot on every refresh, and is a flag *beside*
  `status`, never written into it.
- **A term's reading is the reading of its *headword*.** Sudachi's
  `reading_form` is the reading of the surface, so pairing it with
  `dictionary_form` produced 振る/ふっ and split one verb across a ledger row
  per inflected stem (知る was しる, しら and しっ, each with its own counts and
  its own judgement). `jp_core::tokenize` resolves the lemma's own reading via
  `dictionary_form_word_id`; `POST /api/vocab/repair-readings` folds what the
  old pairing wrote, and is idempotent. Anything keyed on `(headword, reading)`
  — the ledger, `work_terms` — depends on this being right.
- **A tap in the feed judges the word under it**, and is the second writer of
  `vocabulary.status` after `#vocab` — a person tapped a word, so it passes the
  rule below. Two states, because two is all a reader can answer without leaving
  the line: anything marked becomes `known`, and a word already known becomes
  `unknown`. **`new` and `seen` are unreachable by hand and must stay that way** —
  they are what the ledger holds *before* anyone has judged, so writing one
  would be asserting that nothing has been asserted. Tapping past a mistake is
  one more tap, which is why there is no undo and no toast here: the mark is the
  report, it changes under the finger that asked, and a failed write is the mark
  coming back. (The toast still exists for `clear last`, whose undo is the only
  route back to a cleared line.) The judgement applies to every occurrence on
  screen, not the tapped one — one word is one assertion, and leaving the same
  term marked three lines up reads as a write that failed.
  Two things hold it together. It is hit-tested with `caretPositionFromPoint`
  against the text, and **nothing in the feed is made clickable**: an
  interactive layer over the lines would sit between the reader and the text
  Yomitan scans, and a mark that swallowed a long-press would cost a lookup to
  gain a judgement. And a tap that ends a selection is ignored — that is a
  lookup or an explain-focus, not a judgement. A tap on anything unmarked (a
  name, a non-word, a blacklisted term) finds no span and does nothing, which is
  why the `known` spans have to be sent and the rest must not be.
- **Only the reader writes `vocabulary.status`.** Not ingest, not the Anki
  sync, not the lookup sync — a resync must never demote a word marked known,
  and an encounter count must never promote one (`spec/cold-start.md` Pass 4).
  If you add a writer to that column, it needs a person behind it. Today the
  only writers are `/api/vocab/judge` and `/api/vocab/blacklist-non-words`,
  both of which answer a request the reader made. A *reader-triggered* Anki
  import is fine by the same test; folding it into the recurring refresh is not.
- **One word, one row — spelt the way the master dictionary spells it.**
  Terms key on Sudachi's *normalized* form, not its dictionary form, so いう,
  できる, みんな and わかる stop being separate words from 言う, 出来る, 皆 and
  分かる. Where Sudachi and Sankoku disagree about the spelling, Sankoku wins
  (`SudachiTokenizer::written_form`): する normalizes to 為る, which Sankoku
  does not list, and that put the commonest verb in the language outside the
  triage queue with 2,544 encounters.
- **A re-tokenization strands judgements, and the rebuild re-homes them.**
  Moving keys leaves an assertion on a spelling nothing writes to any more.
  `carry_stranded_judgements` asks the tokenizer what each is called now and
  moves the status there, never over the target's own assertion; one with
  nowhere to go is kept, not deleted. The rebuild reports `carried`.
- **A word judged under one reading is not asked about again.** 皆 marked known
  as みな means 皆/みんな is never offered (`triage_queue`) and counts as known
  in the per-work figures (`work_terms::IS_KNOWN`). Both or neither: the ledger
  keys on `(headword, reading)` for the homograph case, but most pairs it
  produces are one word the dictionary lists twice, and asking twice about
  those is what the reader notices.
- **A compound the master dictionary does not list is decomposed into parts it
  does.** Sudachi's splitting stops at its own entries — 懲罰房 has no
  sub-units, so 懲罰 was read 61 times and credited to nothing, while 医務室
  splits fine. `SudachiTokenizer::decompose` longest-matches against Sankoku's
  headwords, and a part must be two characters or a single kanji. **Names are
  never decomposed**: a general dictionary lists no place or surname, so 東京
  looked like an unlistable compound of two words it does list and became
  東 + 京 — twenty-two sightings of "east" and "capital" — while 間宮 gave
  宮 ×95 and 木村 gave 木 ×58. Bare kana is excluded for the same reason in
  miniature: み is a noun, so 楽しみ split into 楽し + み. The cost of both
  guards is real and accepted — Sudachi mis-tags 懲罰房 as a *place name*, so
  懲罰 (61 sightings) is never credited, and 凛と keeps its 凛. A rule that
  told those from 東京 would be tuned to the examples that produced it.
- **And adjacent parts the master dictionary lists as one word are rejoined.**
  The mirror of the above, and for years the missing half: every pass only ever
  *split*, so a compound Sudachi's own lexicon lacks was gone before any of our
  logic saw it. しゃくりあげる is not a Sudachi entry, so Mode C returned
  しゃくり + あげ and the ledger credited しゃくる and 上げる while 噦り上げる —
  which Sankoku lists — was never met once. It was not rare: 570 distinct
  compounds over 1,660 occurrences in the first 14.5k lines (落ち着く, 思い出す,
  振り返る, 見上げる, 巻き込む…), with 317 of their ledger rows sitting at zero
  encounters while their parts collected the sightings.
  `SudachiTokenizer::recompose` joins on either the spelling (振り + 返る) or the
  reading (しゃくり + あげる → 噦り上げる, since the text writes in kana what the
  dictionary spells in kanji). **The reading signal is fenced to verb + verb
  with kana heads**, or そう + する merges into 相する and こと + し into 今年;
  a reading naming two headwords is dropped rather than arbitrated. Content
  words only (ていた reads as 訂 + 板), never a proper noun, and three
  characters minimum. Verified by diffing the token stream of all 14,575 lines
  before and after: 1,530 lines changed, every one a pure regrouping — no
  surface altered, nothing dropped or relabelled.
- **A name is not vocabulary.** Sudachi's 固有名詞 subclass keeps a work's cast
  out of the ledger and `work_terms` (they were the top of every per-work
  unknown list), while `word_days` still counts them — that sink asks what text
  was read. The verdict is per *term* over a whole pass, never per occurrence:
  Sudachi tags a surface inconsistently, and filtering occurrence by occurrence
  kept 79 of ノア's 194, which is worse than either whole answer.
- **Each ingest sink has its own watermark.** One tokenization pass fills
  `word_days`, the ledger and `work_terms`, but `tokenized_through_line_id`,
  `vocab_through_line_id` and `work_terms_through_line_id` move independently
  (and the same three for sessions).
  Both sinks are additive and neither is idempotent, so a row goes to a sink
  only when its id is past *that sink's* mark. That is what lets
  `POST /api/vocab/rebuild` re-derive the ledger from the full history without
  double-counting a single day — and it is the repair path for any future
  re-tokenization.
- **The reading view marks words, and never with markup.** `#read` marks the
  words you have not judged known, and does it without adding a single node to
  the feed: `routes/reader/highlight.rs` sends *offsets* with each streamed
  line, and `paintMarks` draws a rounded rectangle per word into a layer
  **behind** the text, positioned from the client rects of Ranges over it.
  Yomitan scans this DOM, so the one text node per line is not a detail of the
  current implementation — it is the constraint. Drawing behind the text rather
  than through it is what satisfies both halves: the words stay untouched, and
  the marks are real elements, so they take a border radius and horizontal
  padding. (The CSS Custom Highlight API is the other way to tint text without
  touching it, and it was the first implementation — but `::highlight()` takes
  background, colour and text-decoration and nothing else, so its marks can only
  ever be flat bands with square edges.) The layer lives inside the scroll
  container and is measured in content coordinates, so it scrolls with the lines
  and nothing runs on scroll; it repaints on a new line, a font change and a
  resize, which is every way the text can reflow. Offsets are UTF-16 code units
  because that is what a `Range` indexes in, and `renderLine` carries a
  `prettier-ignore` so no reflow can put a whitespace node in front of the text
  and shift every mark on the line.
  **A word judged under one reading is not marked under another**, the same
  rule the triage queue and `work_terms::IS_KNOWN` apply — a mark is a
  question, so it obeys the rule about not asking twice. This is not only the
  言う/ゆう case: Sudachi gives an inflected form the reading of *that form*, so
  通れ arrives as the headword 通る with the reading とおれる, a row of its own
  beside the 通る/とおる the reader marked known. Without the rule the feed marks
  a word they judged, under a spelling they never chose and cannot see. The span
  then points at the row carrying the assertion (`known_readings`), so a tap
  takes back *that* judgement rather than writing to the inflected row and
  leaving the word looking exactly as it did.
  Three tiers are painted and `known` is not one of them — the *absence* of a
  mark is what makes the marks readable — but a `known` span **is** sent, since
  a span is also the region a tap judges and a word just marked known has to
  stay tappable to be taken back. `new` and `seen` split the ledger's `new` on the
  encounter count exactly as the `#vocab` counts do (at 1 rather than 0: ingest
  may already have credited the occurrence being drawn). Names, blacklisted
  terms and non-words are never marked; a word too fresh to have a ledger row is
  tested against the master headword set instead, since the never-before-seen
  word is the one the feature exists to point at. The pipeline is the ingest
  pipeline — same Sudachi, same decompose/recompose — built once on the first
  line that needs it and *not* rebuilt: importing a dictionary changes the
  tints only after a restart.
- **Note ids are epoch milliseconds.** That is why a card's creation time needs
  no extra column, and why the id list is kept sorted.
- **Only engagement actions leave `reader_marks`.** Explain does; clear
  deliberately does not — a mark would re-credit exactly the span it exists to
  remove. Mining needs no mark of its own: the note id is already a timestamp,
  so a mined card is presence by construction.
- **Mining is implicit.** Yomitan's `addNote` goes through `routes/ankiproxy`,
  which fires vn-capture.sh once Anki accepts the note (`auto_capture_on_add`,
  on by default). There is no mine button; a card added anywhere gets its audio
  and screenshot.
- **The chime is the only report a mine gets.** All of the enrichment happens in
  a detached task behind a browser tab nobody is watching, so
  `services::chime::mine_complete` plays once at the end of
  `enrich_added_note` — and only when the capture reported `ok` *and* the
  CompactDef write verified. Keep it that strict: a sound that also plays on a
  half-finished card reports nothing, and silence is the signal to go and look
  at the log. (`JP_TOOLS_MINE_CHIME` overrides the file, empty mines in
  silence; `JP_TOOLS_MINE_CHIME_VOLUME` is a percentage, default 50 — it plays
  beside a VN and belongs underneath it.) This is the `import` X-bell that
  `-silent` removed, made deliberate.
- **A capture is anchored at the add, not at the capture.** vn-capture.sh picks
  the line to cut audio around by reading the newest entry in `lines.log`, so
  anything that delays it re-anchors it onto whatever is on screen by then —
  which is the next line, if you clicked add and read on. The proxy therefore
  stamps `now_ts()` the moment the `addNote` arrives and passes it as
  `VN_ANCHOR_TS`, and passes the note it just created as `VN_NOTE_ID` rather
  than letting the script go looking for the most recently added one. The
  screenshot has no such fix available — it can only show the screen as it is
  when it is taken — so nothing may be awaited in front of the capture. In
  `enrich_added_note` the CompactDef call therefore runs *alongside* it
  (`tokio::join!`) and its Anki write happens after. Keep that shape: putting
  the LLM call first is the original bug, and putting the capture first simply
  moves the delay onto CompactDef, which then lands ten seconds after the add.
  The two `updateNoteFields` are left strictly ordered rather than fired
  together — the capture's inside vn-capture.sh, CompactDef's after the join.
  Two concurrent writes to the same note have not been tested and there is
  nothing to gain by starting.
- **An accepted Anki write is not a stored value.** `updateNoteFields` returns
  `{"result": null, "error": null}` for a write Anki accepted; if the note is
  open in Anki's editor, the editor's next save writes its in-memory copy back
  over it and the field is empty again with nothing logged. This was watched
  happen: a card mined at 12:53:56, its CompactDef written at 12:54:03, the
  note opened in between, the field empty afterwards. So the CompactDef path
  goes through `anki::update_note_field_verified`, which reads the field back
  and returns an error when it did not stick. It deliberately does not retry —
  the editor is still open a second later — so the value is the warning, and
  the fix is behavioural: don't open a freshly mined card for a few seconds, or
  reopen it once the definition lands.
- **`chars` excludes punctuation** (`jp_core::text::chars`), matched to
  texthooker-ui so speeds are comparable with other people's. Both this crate
  and `vn-ws-logger.py` write that column, and startup recounts it.

## Working on it

Don't restart the live stack or touch `~/.local/share/jp-tools` while a VN is
actually being read — it interrupts the session, and a half-written window is
awkward to reason about afterwards. (Restarting `vn-buffer` with Textractor
open is itself safe: the logger closes the WebSocket cleanly on SIGTERM, and
only an abortive disconnect crashes the plugin.) Use an isolated instance
instead:

```sh
scripts/dev-instance.sh run             # :3299, on a frozen copy of the data
scripts/dev-instance.sh snapshot before # record every endpoint
# ...make the change...
scripts/dev-instance.sh check before    # must print IDENTICAL
scripts/dev-instance.sh browser         # the SPA actually renders
```

For a refactor that must not change behaviour, the snapshot diff is the proof —
it caught nothing during the 2026-07 restructure precisely because it was run
after every step.

The browser check exists because the client is unbundled ES modules loaded
straight from disk: a bad import path renders *nothing at all* while every JSON
endpoint still passes. It renders `#kanji`, `#vocab` and `#library` separately
for that reason — their panels are reached from no other tab.

`run` holds the terminal and has no `stop`, so a backgrounded instance outlives
the session and the next `run` refuses with "something is already serving
:3299". Take a free port (`DEV_PORT=3298`) rather than clearing it. **Never
`pkill -f` your way out of that** — the dev instance and the live :3200 service
are the same binary path, so every pattern that matches one matches the other,
and killing the live one interrupts whatever is being read. If you must stop a
specific instance, resolve its PID from the port (`ss -ltnp | grep :3299`).

```sh
cargo test -p read-stats     # 53 unit + 16 integration (tests/api.rs)
```

`tests/api.rs` runs the real router against a throwaway database, which is the
layer to add to when the question is "does the SQL select what the derivation
assumes".

## Frontend notes

- Preact + htm from a CDN import map, no build step. `charts.js` and
  `style.css` are re-export/`@import` facades — add a chart or a sheet there.
- **Never let literal text and `${...}` straddle a line break inside an
  ``html`` `` template.** htm collapses the whitespace and prettier reflows
  freely; that combination silently rendered `snapshot 0 min ago` as
  `snapshot0 minago`. Build the string in JS and interpolate it whole.
- The dashboard polls once and passes the result down. Half the cards are
  different readings of the same days, and independent fetches would show a
  stale streak beside a fresh chart. **Tabs choose what renders, never what is
  fetched** — that is what keeps two tabs from disagreeing about the same day.
- Five tabs, one per question. **Today** — `current-reading.js` over `day.js`:
  what you are reading, then how the day against it went (the goal, the totals,
  the curve and the sittings, all following one date). **Trends** —
  `trends.js`, one range selector over the summary tiles, the daily bars, the
  speed panel and the rate panel. **Library** — `library.js`, two levels:
  the shelf (`works-shelf.js`) lists the works as cards, and opening one
  replaces the tab with that work's own page (`work-detail.js` over
  `GET /api/works/detail?work=<title>` — keyed by title, since nothing upserts
  a `works` row for a title you simply start reading). A work with no reading
  behind it does not appear at all: there is no text until it has been read, so
  a queued title would be a card of blanks. The vocabulary, dialogue and
  log-form cards stay at the shelf level — they are about the reading as a
  whole. `spec/library-rewrite.md` has the phases and what is still to come.
  Logged articles collapse into
  one `Articles` row here (`stats::work::ARTICLES_WORK`) — each keeps its own title and URL on the
  session row, where the day's sittings table shows it. That form has two modes over
  one POST: *pages* estimates chars from a page count (a paper book has no text
  to paste), *paste text* takes the article itself and counts it exactly. The
  preview under the textarea comes from `/api/text/count`, not from a `length`
  in JS — which characters count is a rule that lives in one place, and a
  preview disagreeing with the stored number would be worse than none. **Kanji** — `kanji.js` over
  `/api/kanji`: the grid of every kanji ever read, tinted by encounter count,
  sortable by your own frequency, BCCWJ's, grade or recency, and ringed green
  when a card's target word contains it. Plus the summary tiles, grade coverage
  and discovery per day. A "read often, never carded" list belongs here once the
  ledger is seeded — against `vocabulary.status`, not against the deck.
  **Vocab** — `vocab.js`, two sections over the knowledge
  ledger: the status counts, and `triage.js`, the pass that fills them.
- **`#tokenize` is a page, not a tab** (`panels/tokenize.js` over
  `POST /api/tokenize`), reached from the header beside `📖 read`. Paste text,
  see what the pipeline made of it: the line tinted exactly as the feed tints
  it, and under it every token — including the ones the feed drops, each with
  the rule that dropped it (`grammar`, `name`, `non-word`, `blacklisted`). It
  calls `reader::highlight::analyze`, the same function `spans` is a filter
  over, and that is the whole point: a page for checking the tokenizer that ran
  a second, subtly different pipeline would answer a question nobody asked. It
  writes nothing — no ledger row, no `word_days` count, no presence mark; text
  pasted there was not read.
- **A bulk write shows its rows first.** `blacklist-non-words` judges rows the
  queue never displays, so `GET /api/vocab/non-words` lists them (commonest
  first) and the button only appears once they are on screen. That preview
  immediately earned itself: it was about to blacklist いう×398, できる×183,
  みんな×165 and わかる×120 — the wordhood gate matched dictionary *terms*
  only, and a dictionary lists those in kanji. It now also matches a kana
  headword against dictionary readings, which rescued 621 of 1,088 rows.
- **Triage ticks on two signals, never one** (`vocab.js` → `triage.js`, over
  `vocabulary::preselects_known`). A word is preselected `known` only if it was
  met at least `triage_min_encounters` times **and was never looked up**.
  Encounters alone cannot tell "read straight past it" from "looked it up
  twelve times", and unticked means `unknown` on submit — so a one-signal
  default would write wrong assertions in bulk. The rule lives server-side
  because it decides what gets written and has to be testable without a
  browser; the client only seeds its checkboxes from it. Judging is confined to
  the rows on screen, so an interrupted sweep leaves a resumable queue.
- **The sweep is scoped to what has been read since the last one.**
  `sweep_through_ts` (an internal `read-stats.db` setting, like the ingest
  watermarks) is compared against `vocabulary.last_seen`, so a fortnight's
  reading produces a short batch rather than the standing backlog —
  `spec/periodic-sweep.md`. Three rules hold it together: it **moves on submit,
  never on load**, or an interrupted sweep loses its batch; it moves only for a
  request that asked for it (`advance_sweep`), so a one-off judgement made from
  the unscoped list cannot retire words nobody was shown; and it is a **filter
  and nothing else** — `scoped=0` still reaches every ready row, and the
  scoping judges nothing by itself. `last_seen` answers "met since the mark",
  not "crossed the threshold since the mark", so a declined word returns the
  next time it is read. That over-inclusion is deliberate and cheap;
  `word_days` can answer it exactly if the batches ever come out noisy.
- **Status colour is one scale, in HSL, in `base.css`.** `--vocab-new`,
  `--vocab-seen` and `--vocab-unknown` are written as `hsl()` rather than hex
  because they are generated, not chosen: hue names the status — one each, 211
  blue / 276 violet / 28 amber — and lightness says how loudly, with the dark
  ramp mirrored about the page rather than picked afresh. They were shades of
  one hue first and it did not work: two blues have to be compared, and a mark
  on a line being read has to be recognised instead. Both places that show a status read them —
  the reading view's highlights and the `#vocab` swatches — so the tint under a
  word in the feed is the colour of the pile it is counted in. A fourth status
  follows the two rules; it does not get a colour by eye.
- Selected-state has one vocabulary: `background: var(--meter-track)` with
  primary ink (`.segment-on`, `.toggle-on`, `.tab-on`). Not an accent border,
  not a saturated fill — `--series-1` at full strength is spent on the paused
  alarm alone, and only stays legible as an alarm while nothing else claims
  it.
