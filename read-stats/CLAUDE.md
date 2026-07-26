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
| `knowledge.db` | shared, schema owned by `jp_core::knowledge`: `lines`, `works`, `manual_sessions`, `anki_notes`, `word_days`, `lookups`, and the dictionary cache |
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
  funnel, never inflates the day's per-1000-character rate, and never adds to a
  kanji lookup rate whose denominator the line stream cannot see. The guard is
  at the write and nowhere else — don't add a second filter downstream, and
  don't remove this one and expect the readers to cope. It also means a long
  enough `capture_paused` stops lookups: no lines arrive, so nothing is recent.
- **Anki owns mined-state.** `anki_notes` is a snapshot, replaced wholesale.
  Never write back.
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
endpoint still passes.

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
- Four tabs, one per question. **Today** — `current-reading.js` over `day.js`:
  what you are reading, then how the day against it went (the goal, the totals,
  the curve and the sittings, all following one date). **Trends** —
  `trends.js`, one range selector over the summary tiles, the daily bars, the
  speed panel and the rate panel. **Library** — `library.js`: works,
  vocabulary, dialogue, and the manual log form. **Kanji** — `kanji.js` over
  `/api/kanji`: the grid of every kanji ever read, tinted by encounter count,
  sortable by your own frequency or BCCWJ's, and ringed twice — green for a
  card's target word, red for a lookup rate past three times your own average
  over six-plus readings (`stats::kanji::OUTLIER_*`, applied server-side so the
  legend can state the threshold it used). A ring for "has been looked up at
  all" was the first attempt and said nothing: one lookup is what reading is.
  Plus grade coverage, the corpus-coverage curve,
  discovery per day, the lookup-rate ranking and the per-work fingerprints. The
  page was twelve cards in one column before, with today and the last 30 days
  interleaved and four slices of the same window each carrying its own
  hardcoded range.
- Selected-state has one vocabulary: `background: var(--meter-track)` with
  primary ink (`.segment-on`, `.toggle-on`, `.tab-on`). Not an accent border,
  not a saturated fill — `--series-1` at full strength is spent on the paused
  alarm alone, and only stays legible as an alarm while nothing else claims
  it.
