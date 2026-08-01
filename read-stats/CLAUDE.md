# read-stats — daily reading tracker + the `#read` reading view

Rust 2024. Axum JSON API + Preact/htm frontend (no build step), two SQLite
databases. Port 3200.

- **the dashboard** — how much was read, how fast, how continuously, what it
  cost in lookups. Everything is derived from the raw line stream at query time,
  so changing a threshold re-reads the whole history under the new rule.
- **`#read`** — the live line feed read beside the running VN, the explain
  button, and the AnkiConnect proxy Yomitan points at. Served over LAN and
  Tailscale too, so a second device beside the screen works the same way.

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

## Where to start

`src/lib.rs` is the layer map. Then `stats/presence.rs` — how much of a gap
counts as reading is the decision everything else builds on. `clock.rs` holds
the only impure inputs (`now_ts`, `tz_offset_secs`); each `db/` module doc says
which database it talks to.

## Two databases

| | |
|---|---|
| `knowledge.db` | shared, schema owned by `jp_core::knowledge`: `lines`, `works`, `manual_sessions`, `anki_notes`, `word_days`, `lookups`, `vocabulary`, and the dictionary cache |
| `read-stats.db` | this app's own: `settings`, `reader_marks`, `work_covers` |

The split is `spec/knowledge-db.md`'s: what is about the reading is shared, what
is about this app's behaviour is local. `db` functions take a `Knowledge` handle
or a bare `SqlitePool`, so passing the wrong database is a compile error.

Two places straddle the line — the current work's capture window and the cover
sources — and both join in memory. Keep it that way; a cross-database join here
would also have to be taught to `vn-capture.sh`.

## Invariants worth knowing before changing anything

- **Presence is the rule everything credits time through.** A new aggregate that
  measures time goes through `stats::Presence`, not a fresh `min(gap, cap)`.
  When those two diverged, the focus metric punished the reader for using a
  dictionary.
- **Pace is a property of the reader, not of a request.** `History` derives it
  once over all history. Per-endpoint derivation made the dashboard and the day
  timeline disagree about the same day.
- **Nothing is deleted.** A line that shouldn't count gets `discarded = 1`,
  filtered on read.
- **Pausing stops capture, it does not filter.** `settings.capture_paused` is
  polled by vn-ws-logger.py, which closes its Textractor WebSocket while it is
  set. A paused span has no lines in it, so nothing needs excluding. The old
  `pauses` table is retired on startup by `db::retire_pauses`.
- **A lookup only exists if it happened while reading.** Yomitan points at the
  proxy from the browser, so it fires for anything looked up anywhere.
  `ankiproxy::record` records only when a line arrived within
  `session_gap_secs`, so a news article never enters a VN's funnel. The guard is
  at the write and nowhere else — don't add a second filter downstream. It also
  means a long enough `capture_paused` stops lookups.
- **Exposure counts take all text; cost counts take only hooked text.** Pasted
  session `content` feeds `word_days`, the kanji grid, the discovery curve and
  every coverage figure. It stays out of every rate: `lookups_per_1k` divides by
  hooked characters (`stats::rate`). Article characters could enter a rate's
  denominator but never its numerator. Decide which of the two questions a new
  figure asks before choosing its denominator.
- **Speed divides by measured reading only** (`History::measured_days`). An
  untimed session's duration is derived from the reader's own pace, so in a
  speed chart it would measure its own output. Totals, goals and streaks still
  count everything read.
- **Anki owns mined-state.** `anki_notes` is a snapshot, replaced wholesale,
  never written back. `vocabulary.mined` is recomputed from it on every refresh
  and is a flag beside `status`, never written into it.
- **A term's reading is the reading of its headword.** Sudachi's `reading_form`
  is the reading of the surface, so pairing it with `dictionary_form` produced
  振る/ふっ and split 知る across しる, しら and しっ, each with its own counts
  and judgement. `jp_core::tokenize` resolves the lemma's reading via
  `dictionary_form_word_id`. `POST /api/vocab/repair-readings` folds what the
  old pairing wrote and is idempotent. The ledger and `work_terms` depend on it.
- **A tap in the feed judges the word under it.** Two states: anything marked
  becomes `known`, a word already known becomes `unknown`. **`new` and `seen`
  are unreachable by hand and must stay that way** — they are what the ledger
  holds before anyone has judged. No undo and no toast: the mark is the report,
  it changes under the finger, and a failed write is the mark coming back. (The
  toast stays for `clear last`, whose undo is the only route back to a cleared
  line.) The judgement applies to every occurrence on screen, not the tapped
  one.
  Hit-tested with `caretPositionFromPoint` against the text, and **nothing in
  the feed is made clickable** — an interactive layer would sit between the
  reader and the text Yomitan scans, and a mark that swallowed a long-press
  would cost a lookup to gain a judgement. A tap ending a selection is ignored
  (that is a lookup or an explain-focus). A tap on anything unmarked finds no
  span and does nothing, which is why `known` spans are sent and the rest are
  not.
- **Only the reader writes `vocabulary.status`.** Not ingest, not the Anki sync,
  not the lookup sync — a resync must never demote a word marked known, and an
  encounter count must never promote one (`spec/cold-start.md` Pass 4). A writer
  to that column needs a person behind it. Today: `/api/vocab/judge`,
  `/api/vocab/blacklist-non-words`, the tap in the feed, and the imports
  (`anki-import`, `jiten-import`, frequency). A reader-triggered import passes
  that test; folding the same logic into the recurring Anki refresh does not.
- **One word, one row — spelt the way the master dictionary spells it.** Terms
  key on Sudachi's *normalized* form, so いう, できる, みんな and わかる are not
  separate from 言う, 出来る, 皆 and 分かる. Where Sudachi and Sankoku disagree,
  Sankoku wins (`SudachiTokenizer::written_form`): する normalizes to 為る, which
  Sankoku does not list, which put the commonest verb in the language outside
  the triage queue with 2,544 encounters.
- **A re-tokenization strands judgements, and the rebuild re-homes them.**
  `carry_stranded_judgements` asks the tokenizer what each is called now and
  moves the status there, never over the target's own assertion; one with
  nowhere to go is kept, not deleted. The rebuild reports `carried`.
- **A word judged under one reading is not asked about again.** 皆 marked known
  as みな means 皆/みんな is never offered (`triage_queue`) and counts as known
  in the per-work figures (`work_terms::IS_KNOWN`). The ledger keys on
  `(headword, reading)` for the homograph case, but most pairs it produces are
  one word the dictionary lists twice.
- **A compound the master dictionary does not list is decomposed into parts it
  does.** Sudachi's splitting stops at its own entries — 懲罰房 has no sub-units,
  so 懲罰 was read 61 times and credited to nothing, while 医務室 splits fine.
  `SudachiTokenizer::decompose` longest-matches against Sankoku's headwords; a
  part must be two characters or a single kanji. **Names are never decomposed**:
  東京 became 東 + 京 (twenty-two sightings of "east" and "capital"), 間宮 gave
  宮 ×95, 木村 gave 木 ×58. Bare kana is excluded for the same reason: み is a
  noun, so 楽しみ split into 楽し + み. Both guards cost something and it is
  accepted — Sudachi mis-tags 懲罰房 as a place name, so 懲罰's 61 sightings are
  never credited, and 凛と keeps its 凛.
- **Adjacent parts the master dictionary lists as one word are rejoined.** The
  mirror of the above. しゃくりあげる is not a Sudachi entry, so Mode C returned
  しゃくり + あげ and credited しゃくる and 上げる while 噦り上げる — which
  Sankoku lists — was never met once. 570 distinct compounds over 1,660
  occurrences in the first 14.5k lines (落ち着く, 思い出す, 振り返る, 見上げる,
  巻き込む…), with 317 of their ledger rows at zero encounters.
  `SudachiTokenizer::recompose` joins on the spelling (振り + 返る) or the
  reading (しゃくり + あげる → 噦り上げる). **The reading signal is fenced to
  verb + verb with kana heads**, or そう + する merges into 相する and こと + し
  into 今年; a reading naming two headwords is dropped rather than arbitrated.
  Content words only (ていた reads as 訂 + 板), never a proper noun, three
  characters minimum. Verified by diffing the token stream of all 14,575 lines:
  1,530 lines changed, every one a pure regrouping.
- **An affix the master dictionary lists is a word.** Sudachi tags the trailing
  達 of 私達 as 接尾辞 and the content-word gate threw it away, so 私達 was
  decomposed into 私 + 達 and half of it credited to nothing — the same defect as
  懲罰房, arriving through the part-of-speech tag instead.
  `jp_core::tokenize::counts_as_word` admits 接尾辞/接頭辞 when the master lists
  the `(headword, reading)` pair — the pair, because 鬼/き and 鬼/おに are both
  Sankoku entries. That test is the whole fence: it admits 達/たち, 御/お,
  的/てき, 鬼/き and refuses げ, ぷ, さん/さーん, 日/じつ (40 terms over 198
  occurrences in the first 16,325 lines) with no stoplist to maintain. Ingest
  and the highlighter ask the identical question, so a tint and a ledger row
  cannot disagree about 達. Cost: 232 terms / 5,542 occurrences enter the three
  sinks, led by ちゃん×1461, さん×807, 達×520, 御×390, tinted on nearly every
  line until judged once.
- **A name is not vocabulary.** Sudachi's 固有名詞 subclass keeps a work's cast
  out of the ledger and `work_terms`; `word_days` still counts them, since that
  sink asks what text was read. The verdict is per *term* over a whole pass,
  never per occurrence — Sudachi tags a surface inconsistently, and filtering
  occurrence by occurrence kept 79 of ノア's 194.
- **Each ingest sink has its own watermark.** One pass fills `word_days`, the
  ledger and `work_terms`, but `tokenized_through_line_id`,
  `vocab_through_line_id` and `work_terms_through_line_id` move independently
  (same three for sessions). The sinks are additive and not idempotent, so a row
  goes to a sink only when its id is past *that sink's* mark. That is what lets
  `POST /api/vocab/rebuild` re-derive the ledger from the full history without
  double-counting a day, and it is the repair path for any re-tokenization.
- **The reading view marks words, and never with markup.**
  `routes/reader/highlight.rs` sends offsets with each streamed line, and
  `paintMarks` draws a rounded rectangle per word into a layer **behind** the
  text, positioned from the client rects of Ranges over it. Yomitan scans this
  DOM, so one text node per line is a constraint, not an implementation detail.
  Drawing behind the text keeps the words untouched while the marks stay real
  elements, so they take a border radius and padding. (The CSS Custom Highlight
  API was the first implementation, but `::highlight()` takes background, colour
  and text-decoration only — flat bands with square edges.) The layer sits
  inside the scroll container in content coordinates, so nothing runs on scroll;
  it repaints on a new line, a font change and a resize. Offsets are UTF-16 code
  units because that is what a `Range` indexes in, and `renderLine` carries a
  `prettier-ignore` so no reflow can put a whitespace node in front of the text.
  **A word judged under one reading is not marked under another**, the same rule
  the triage queue applies. Not only the 言う/ゆう case: Sudachi gives an
  inflected form the reading of that form, so 通れ arrives as 通る/とおれる, a row
  of its own beside the 通る/とおる the reader marked known. The span points at
  the row carrying the assertion (`known_readings`), so a tap takes back *that*
  judgement rather than writing to the inflected row.
  Three tiers are painted and `known` is not one of them — the absence of a mark
  is what makes the marks readable — but a `known` span **is** sent, since a
  span is also the region a tap judges. `new` and `seen` split the ledger's `new`
  on encounter count as the `#vocab` counts do (at 1 rather than 0: ingest may
  already have credited the occurrence being drawn). Names, blacklisted terms
  and non-words are never marked; a word too fresh to have a ledger row is
  tested against the master headword set instead. The pipeline is the ingest
  pipeline, built once on the first line that needs it and not rebuilt —
  importing a dictionary changes the tints only after a restart.
- **The feed re-pins to the bottom on a new *line*, not on a new `lines`.** The
  stick-to-bottom effect keys on the id of the newest line on screen, because
  judging a word rebuilds the array without adding to it. Keyed on `lines` it
  fired on every tap: scroll back a little (still inside `STICK_SLOP_PX`), tap a
  word, and the feed jumped to the bottom, taking the word out from under the
  finger. Prepended history is excluded by the same key; `loadMoreHistory`
  restores the position itself.
  **A reflow re-pins too, on a height test rather than an id** (`pinToBottom`).
  An id-keyed pin cannot see the feed move under a reader who never touched it,
  and three things move it on an ordinary load: the web font landing after first
  paint (`display=swap`, so every line is measured twice), a page of history
  arriving on top, and the pane being resized. Cold, that opened the feed 1500px
  from the bottom and drifted further with each page. Judging a word changes no
  height, so a tap is a no-op here and the word stays under the finger.
- **The `◌ marked` filter is a view, never a write.** It hides every line with
  nothing marked in it, for scrolling back over a finished sitting. It filters
  on **membership, not a live predicate**: `keptIds` holds every line that has
  *had* a marked word since the filter was switched on, grows as marked lines
  arrive, and never shrinks. Under a live predicate, judging the last marked
  word in a line deleted that line from under the finger that judged it. The
  line staying, unmarked, is also the report. Toggling the filter off and on is
  the only thing that clears judged lines out.
  `lines` stays the whole feed and the filter is applied at the last moment
  (`visible`), so a judgement rewrites hidden occurrences too — a mark
  reappearing when the filter comes off would read as a failed write. Two
  consequences: everything that measures or hit-tests the text (`paintMarks`,
  `spanAtPoint`) takes `visible`, since a hidden line has no element to range
  over; and because `visible` derives from `keptIds`, the repaint must **depend
  on `keptIds`, not only on `lines`**. `keptIds` settles a render later, which
  stranded every mark on a backscroll — the prepended page grew `lines` and
  repainted against the old `keptIds`, then the effect admitting them to the
  filter pushed everything on screen down with nothing in the dependency list
  changed to repaint it. Marks sat a page-height off their words until an
  unrelated repaint corrected them.
  `clear last` is disabled while the filter is on: it drops the newest line,
  which the filter may be hiding. A line counts as marked by the painted tiers
  only — `known` spans are sent for every judged word, so counting any token
  would keep nearly every line. The automatic backscroll top-up runs **on a
  budget** (`FILTERED_TOPUP_PAGES`) rather than not at all: an uncapped "pull
  until it scrolls" loop would page back through the entire history, but off
  entirely was worse — a feed that cannot scroll cannot reach the scroll
  trigger, and a just-started sitting could leave the view stuck on one line
  until the filter was turned off. The budget resets each time the filter is
  turned on.
- **Note ids are epoch milliseconds.** That is why a card's creation time needs
  no extra column, and why the id list is kept sorted.
- **Only engagement actions leave `reader_marks`.** Explain does; clear does not
  — a mark would re-credit exactly the span it exists to remove. Mining needs no
  mark: the note id is already a timestamp.
- **Mining is implicit.** Yomitan's `addNote` goes through `routes/ankiproxy`,
  which fires vn-capture.sh once Anki accepts the note (`auto_capture_on_add`,
  on by default). There is no mine button; a card added anywhere gets its audio
  and screenshot.
- **The chime is the only report a mine gets.** Enrichment happens in a detached
  task behind a tab nobody is watching, so `services::chime::mine_complete`
  plays at the end of `enrich_added_note`, and only when the capture reported
  `ok` *and* the CompactDef write verified. Keep it that strict: a sound that
  also plays on a half-finished card reports nothing, and silence is the signal
  to check the log. (`JP_TOOLS_MINE_CHIME` overrides the file, empty mines in
  silence; `JP_TOOLS_MINE_CHIME_VOLUME` is a percentage, default 50.)
- **A capture is anchored at the add, not at the capture.** vn-capture.sh picks
  the line to cut audio around from the newest entry in `lines.log`, so anything
  that delays it re-anchors it onto whatever is on screen by then. The proxy
  stamps `now_ts()` when the `addNote` arrives and passes it as `VN_ANCHOR_TS`,
  and passes the note it created as `VN_NOTE_ID`. The screenshot has no such fix
  — it shows the screen as it is when taken — so nothing may be awaited in front
  of the capture. In `enrich_added_note` the CompactDef call runs *alongside* it
  (`tokio::join!`) with its Anki write after. Keep that shape: the LLM call first
  is the original bug, and the capture first moves the delay onto CompactDef,
  which then lands ten seconds after the add. The two `updateNoteFields` stay
  strictly ordered — two concurrent writes to the same note are untested and
  there is nothing to gain by starting.
- **An accepted Anki write is not a stored value.** `updateNoteFields` returns
  `{"result": null, "error": null}` for a write Anki accepted; if the note is
  open in Anki's editor, the editor's next save writes its in-memory copy back
  over it, with nothing logged. Watched happen: card mined 12:53:56, CompactDef
  written 12:54:03, note opened in between, field empty afterwards. The
  CompactDef path goes through `anki::update_note_field_verified`, which reads
  the field back and errors when it did not stick. It does not retry — the
  editor is still open a second later — so the fix is behavioural: don't open a
  freshly mined card for a few seconds.
- **`chars` excludes punctuation** (`jp_core::text::chars`), matched to
  texthooker-ui so speeds are comparable with other people's. Both this crate
  and `vn-ws-logger.py` write that column, and startup recounts it.

## Working on it

Don't restart the live stack or touch `~/.local/share/jp-tools` while a VN is
being read — it interrupts the session. (Restarting `vn-buffer` with Textractor
open is safe: the logger closes the WebSocket cleanly on SIGTERM, and only an
abortive disconnect crashes the plugin.) Use an isolated instance:

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
endpoint still passes. It renders `#kanji`, `#vocab` and `#library` separately,
since their panels are reached from no other tab.

`run` holds the terminal and has no `stop`, so a backgrounded instance outlives
the session and the next `run` refuses with "something is already serving
:3299". Take a free port (`DEV_PORT=3298`) rather than clearing it. **Never
`pkill -f` your way out of that** — the dev instance and the live :3200 service
are the same binary path, so every pattern matching one matches the other. To
stop a specific instance, resolve its PID from the port (`ss -ltnp | grep :3299`).

```sh
cargo test -p read-stats     # 76 unit + 38 integration (tests/api.rs)
```

`tests/api.rs` runs the real router against a throwaway database — the layer to
add to when the question is "does the SQL select what the derivation assumes".

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
  fetched.**
- Five tabs, one per question.
  - **Today** — `current-reading.js` over `day.js`: what you are reading, then
    how the day went (goal, totals, curve, sittings, all following one date).
  - **Trends** — `trends.js`: one range selector over the summary tiles, daily
    bars, speed panel and rate panel.
  - **Library** — `library.js`, two levels. The shelf (`works-shelf.js`) lists
    works as cards; opening one replaces the tab with that work's page
    (`work-detail.js` over `GET /api/works/detail?work=<title>`, keyed by title
    since nothing upserts a `works` row for a title you simply start reading). A
    work with no reading behind it does not appear — there is no text until it
    has been read. The vocabulary, dialogue and log-form cards stay at shelf
    level. `spec/library-rewrite.md` has what is still to come.
    Logged articles collapse into one `Articles` row (`stats::work::ARTICLES_WORK`);
    each keeps its title and URL on the session row, shown in the day's sittings
    table. The log form has two modes over one POST: *pages* estimates chars
    from a page count, *paste text* counts the article exactly. The preview
    comes from `/api/text/count`, not a `length` in JS — which characters count
    is a rule that lives in one place.
  - **Kanji** — `kanji.js` over `/api/kanji`: every kanji ever read, tinted by
    encounter count, sortable by your own frequency, BCCWJ's, grade or recency,
    ringed green when a card's target word contains it. Plus summary tiles,
    grade coverage and discovery per day. A "read often, never carded" list
    belongs here once the ledger is seeded — against `vocabulary.status`, not
    against the deck.
  - **Vocab** — `vocab.js`: the status counts, and `triage.js`, the pass that
    fills them.
- **`#tokenize` reports the tokenizer, not the ledger's folding.**
  `Analyzed.reading` is the reading the token was produced with; where the
  status came from a different row for the same headword, `judged_as` carries
  that row's reading in its own column. The feed still folds them (a span has to
  point at the row a tap takes back) and `spans` is where that happens, on the
  way out of `analyze`. Overwriting the reading upstream of both made the page
  report 鬼 as おに inside 殺人鬼 where the tokenizer said き — and the two are
  separate Sankoku entries.
- **`#tokenize` is a page, not a tab** (`panels/tokenize.js` over
  `POST /api/tokenize`), reached from the header beside `📖 read`. Paste text,
  see what the pipeline made of it: the line tinted as the feed tints it, and
  every token under it — including the ones the feed drops, each with the rule
  that dropped it (`grammar`, `name`, `non-word`, `blacklisted`). It calls
  `reader::highlight::analyze`, the same function `spans` filters over. It
  writes nothing — no ledger row, no `word_days` count, no presence mark.
- **A bulk write shows its rows first.** `blacklist-non-words` judges rows the
  queue never displays, so `GET /api/vocab/non-words` lists them (commonest
  first) and the button only appears once they are on screen. The preview
  immediately earned itself: it was about to blacklist いう×398, できる×183,
  みんな×165 and わかる×120 — the wordhood gate matched dictionary *terms* only,
  and a dictionary lists those in kanji. It now also matches a kana headword
  against dictionary readings, which rescued 621 of 1,088 rows.
- **Triage ticks on two signals, never one** (`triage.js` over
  `vocabulary::preselects_known`). A word is preselected `known` only if it was
  met at least `triage_min_encounters` times **and was never looked up**.
  Encounters alone cannot tell "read straight past it" from "looked it up twelve
  times", and unticked means `unknown` on submit, so a one-signal default would
  write wrong assertions in bulk. The rule lives server-side because it decides
  what gets written; the client only seeds its checkboxes from it. Judging is
  confined to the rows on screen, so an interrupted sweep leaves a resumable
  queue.
- **The sweep is scoped to what has been read since the last one.**
  `sweep_through_ts` (a `read-stats.db` setting) is compared against
  `vocabulary.last_seen`, so a fortnight's reading produces a short batch rather
  than the standing backlog — `spec/periodic-sweep.md`. Three rules: it **moves
  on submit, never on load**, or an interrupted sweep loses its batch; it moves
  only for a request that asked (`advance_sweep`), so a one-off judgement from
  the unscoped list cannot retire words nobody was shown; and it is a **filter
  and nothing else** — `scoped=0` still reaches every ready row. `last_seen`
  answers "met since the mark", not "crossed the threshold since the mark", so a
  declined word returns the next time it is read. That over-inclusion is cheap;
  `word_days` can answer it exactly if the batches come out noisy.
- **Status colour is one scale, in HSL, in `base.css`.** `--vocab-new`,
  `--vocab-seen` and `--vocab-unknown` are `hsl()` rather than hex because they
  are generated, not chosen: hue names the status (211 blue / 276 violet / 28
  amber), lightness says how loudly, and the dark ramp mirrors the light one.
  Shades of one hue did not work — two blues have to be compared, where a mark
  on a line being read has to be recognised. Both places that show a status read
  these, so the tint under a word in the feed is the colour of the pile it is
  counted in. A fourth status follows the two rules; it does not get a colour by
  eye.
- Selected-state has one vocabulary: `background: var(--meter-track)` with
  primary ink (`.segment-on`, `.toggle-on`, `.tab-on`). Not an accent border,
  not a saturated fill — `--series-1` at full strength is spent on the paused
  alarm alone, and only stays legible as an alarm while nothing else claims it.
