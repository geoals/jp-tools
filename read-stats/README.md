# read-stats

Automatic daily reading tracker: characters read and active reading time,
derived from the raw line stream `vn-mine/vn-ws-logger.py` already captures —
no manual copying, no counters to reset. Dashboard with goal meter (floor /
target minutes), streak, daily-minutes chart, a chars/hour trend, and a
toggleable lookups/h vs cards/h trend.

## How it works

- **Ingestion is passive.** `vn-ws-logger.py` (running under the `vn-buffer`
  systemd unit) inserts every hooked line — timestamp, char count, text — into
  `~/.local/share/jp-stats/stats.db`. The web service only reads that DB, so
  stats are captured whenever you read, whether or not the dashboard is
  running.
- **Characters are counted like texthooker-ui does** (`charcount.rs`, mirrored
  in `vn-ws-logger.py`): an allowlist of kana, kanji, radicals and
  alphanumerics, so punctuation and brackets don't inflate chars/h. Startup
  recomputes `lines.chars` for any row that disagrees, so a change to the rule
  (or a logger still running an older one) is corrected on the next restart.
- **Everything is derived at query time** from raw line events, so thresholds
  are tunable after the fact:
  - a gap between lines credits reading time, capped at `afk_secs` (30). Set
    from the measured gap distribution rather than by feel: gaps containing a
    Yomitan lookup cluster at 10–32s (median 24s, p90 32s) while gaps without
    one have a p90 of 9s, so 30 keeps a real lookup whole and truncates the
    tail where a lookup became a distraction. At 20 the majority of lookups
    were being clipped, inflating chars/h by ~6%;
  - a gap over `session_gap_secs` (600) closes the session;
  - days roll over at `day_rollover_hour` (04:00) — late-night reading counts
    toward the evening's day.
- **Yomitan lookups are counted by proxying AnkiConnect** (`ankiproxy.rs`).
  Yomitan checks Anki for duplicates every time it shows a definition popup, so
  with its server address pointed at `/anki-proxy` each popup becomes a row in
  `lookups`. Requests are forwarded to the real AnkiConnect byte-for-byte, so
  mining is unaffected; a lookup is recorded before forwarding, so it still
  counts when Anki is closed. Repeated requests for the same term within 3s
  collapse into one lookup (a single popup fires several). See *Counting
  lookups* below.
- **Focus measures how continuous the reading was**, not how much of it there
  was (`stats.rs::aggregate_focus_days`). Active time caps each gap at
  `afk_secs`, which is precisely what hides fragmentation — so focus keeps the
  *uncapped* span beside it and reports `active / span`. 100% means every gap
  was a normal reading beat; 60% means two fifths of your at-desk time went
  somewhere else. Gaps over `session_gap_secs` are excluded (that's leaving, not
  being distracted); gaps over 60s count as interruptions and break the
  longest-stretch run. Needs the line stream, so manually logged sessions have
  no focus figure.
- **Pause** (`POST /api/pause` toggle, dashboard button, or bind
  `jp-stats-pause.sh` to a hotkey) for skipping scenes / replaying read text:
  lines are still captured raw but derivation ignores those inside a pause
  interval, so a forgotten pause can be fixed retroactively by editing the
  `pauses` table.
- **Manual sessions** cover everything without a line stream: physical books
  (pages × `chars_per_page`, default 550 ≈ bunkobon), manga, or imported
  history. Logged from the dashboard form or `POST /api/sessions`.
- **Work metadata** turns per-work totals into progress. A work row (keyed by
  the exact title stamped on lines/sessions) carries a `total_chars` count
  pasted manually from the VN's jpdb page (jpdb has no public API) and
  optionally a cover: pass a VNDB id once and the cover is fetched from
  `api.vndb.org/kana`, cached next to the DB in `covers/`, served at
  `/covers/` — nothing else from VNDB is stored. The currently-reading card
  shows cover, char-based progress bar, hours left at the work's own speed,
  and a projected finish date from the trailing 7 complete days' pace
  (clipped to the `pace_start_date` setting, for coming back from a break).

- **Anki integration (read-only).** On dashboard load (or the refresh button)
  the server probes for AnkiConnect — the dashboard client's IP first (phone
  running AnkiconnectAndroid), then `JP_TOOLS_ANKI_URL` — and snapshots the
  mined deck's `VocabKanji` fields into `anki_notes`. Note ids double as
  creation timestamps, giving **cards per session** (cards added inside each
  session's timespan, cards/h) with no extra bookkeeping. New raw lines are
  tokenized incrementally (jp-core Sudachi, mined vocab as validation
  headwords) into per-day lemma counts (`word_days`), which power the
  **re-encounter card**: how many mined words you've since met again in real
  reading, this week's most-met words, and mined-but-never-re-encountered
  words. `word_days` is deck-independent, so words mined later still match
  past reading.

## Reading from a phone (`/#read`)

`#read` is a live feed of the lines Textractor hooks, meant for reading a VN on
the PC while the phone does the looking-up. The setup:

- **Sunshine** on the PC + **Moonlight** on the phone streams the VN. Put the
  two in Android split-screen — Moonlight above, Firefox on `#read` below — so
  the lines are always visible and there is no app-switching per lookup.
- **Yomitan in Firefox Android** scans the lines. Point its *Server address* at
  `http://<pc-ip>:3200/anki-proxy` exactly as on the desktop, so phone lookups
  are counted too and cards land in the **PC's** Anki.
- **⛏ mine last line** runs `vn-mine/vn-capture.sh` on the PC, attaching the
  voiceline audio and a screenshot to the note Yomitan just added, and reports
  the outcome on the page instead of via `notify-send` on a desktop nobody is
  looking at.

Cards must land in the PC's collection, not the phone's, because that is what
`vn-capture.sh` attaches media to — so the proxy forwards to `JP_TOOLS_ANKI_URL`
unconditionally, deliberately *not* preferring the requesting client the way
manga-mine's export does.

Input from Moonlight goes to the VN, so the VN keeps desktop focus throughout
and the usual "click back to the VN window first" caveat doesn't apply. The
5-minute ring-buffer limit still does: mine before advancing.

The line feed is read from the `lines` table that `vn-ws-logger.py` already
writes, not from Textractor's WebSocket — its plugin can crash Textractor when a
client disconnects abortively, so a second WS client would be a risk for nothing.

## Run

```sh
cargo run -p read-stats     # http://localhost:3200
```

Or as part of the stack: `scripts/start-all.sh`.

## API

- `GET  /api/summary` — today (chars, active seconds, per-source, lookups),
  goal, streak
- `GET  /api/days?days=60` — zero-filled per-day totals, oldest first
- `POST /anki-proxy` — AnkiConnect pass-through that counts Yomitan lookups;
  point Yomitan's server address here (see *Counting lookups*)
- `GET  /api/sessions?date=2026-07-19` — derived VN sessions + manual sessions
- `POST /api/sessions` — `{date?, start_ts?, minutes, chars? | pages?, work?, source?, note?}`
- `DELETE /api/sessions/{id}`
- `GET  /api/works` — per-work (title) totals, merging line-stream and manual
  sessions, each with its metadata (`meta`: vndb id, total_chars, cover, status);
  metadata-only works (e.g. queued) get a zero row
- `POST /api/works` — create/update metadata by title:
  `{title, vndb_id?, total_chars?, status?, queue_pos?}` — `vndb_id` accepts
  `v3144` / `3144` / a vndb.org URL, is used once to fetch the cover and not
  stored; empty string removes the cover; `total_chars: 0` clears; status ∈
  reading/queued/finished/dropped
- `PUT  /api/works/{id}` / `DELETE /api/works/{id}` — same fields by id / remove
- `GET  /api/lines/stream` — SSE, one event per hooked line, `data` being
  `{id, ts, chars, text}` and the event id being the line id. Sends the last
  `?backlog=` lines (40) on open, or resumes after `?after=<id>` /
  `Last-Event-ID` so a reconnecting phone doesn't replay or skip
- `GET  /api/reader/state` — `{paused, current_work, capture_available}`
- `POST /api/vn/capture` — run `vn-capture.sh` (see `JP_TOOLS_VN_CAPTURE_SH`)
  and return its result. A capture that fails for an ordinary reason (stale
  line, Anki closed) is `200 {"ok": false, "error": ...}`; only an unrunnable
  or unparseable script is a 5xx
- `POST /api/pause` — toggle an open-ended pause interval
- `POST /api/anki/refresh` — probe AnkiConnect (client IP, then fallback),
  snapshot the deck, tokenize new lines
- `GET  /api/anki/summary` — mined count, re-encountered count, 7-day
  encounters, top words, never-re-encountered sample
- `GET  /api/lookups/summary` — lookup outcomes per distinct term (mined /
  already-carded / never carded), repeat-lookup list, leech list, median
  lookup→card latency
- `GET/PUT /api/settings` — `afk_secs`, `session_gap_secs`, `day_rollover_hour`,
  `goal_floor_mins`, `goal_target_mins`, `chars_per_page`, `current_work`,
  `pace_start_date` (ISO date or "" — clips the finish-estimate pace window;
  no dashboard control, set it here after a reading break:
  `curl -X PUT localhost:3200/api/settings -H 'Content-Type: application/json'
  -d '{"pace_start_date": "2026-07-15"}'`)

### Counting lookups

In Yomitan → Settings → Anki, set **Server address** to:

```
http://127.0.0.1:3200/anki-proxy
```

Requirements: *Enable Anki integration* on and *Check for duplicate cards* on
(the default) — the duplicate check is the signal. Nothing else changes; cards
are still added through the same path, and read-stats' own AnkiConnect calls
bypass the proxy so a refresh can't inflate the count.

Yomitan's duplicate check uses the **first field** of the note type, which must
be the field named in `JP_TOOLS_ANKI_FIELD_VOCAB` (`VocabKanji`) for the term to
be recorded. To confirm it's working, do a lookup and:

```sh
sqlite3 ~/.local/share/jp-stats/stats.db 'SELECT ts, term, work FROM lookups ORDER BY id DESC LIMIT 5;'
```

An empty table with popups appearing means the request shape wasn't recognized —
run the server with `RUST_LOG=read_stats=debug` and look for
`lookup action with no extractable term`.

### What lookups turn into

The *Lookups* card classifies each distinct looked-up term by comparing the
card's creation time (the Anki note id is epoch ms) against the term's first
lookup:

- **became cards** — a card was made at or after the lookup; the lookup stuck.
- **already had a card** — a card existed *before* the lookup: a word that was
  mined but didn't take. These are leeches, listed with the card's age.
- **repeat lookups** — the same word looked up more than once, worst first, each
  tagged with its outcome. An unmined repeat is a mining candidate; a carded
  repeat is a card that isn't working.

Counts are over distinct terms, not lookup events, so a word looked up five
times before being mined counts once and can't inflate the rate. All of it joins
`lookups.term` to `anki_notes.vocab`, and `anki_notes` is a **snapshot** — run
`POST /api/anki/refresh` (or the dashboard's ↻) first or anything mined since
the last refresh reads as "never carded".

`lookups/1k` in the recent-days table is lookups per 1000 characters: the
unknown-word rate, suppressed below 500 chars/day where the ratio is mostly
noise. The *Lookups & cards* chart plots lookups/h against mined cards/h — both
are events per hour, so they share one y-axis and either can be toggled off from
the legend. Days under 10 minutes read are omitted: the per-hour denominator is
too small to mean anything. Minutes read deliberately stays in its own chart
rather than being overlaid here, since a second y-scale would imply a
correlation the data doesn't contain.

### Importing spreadsheet history

One manual session per historical day carries old totals into streaks and
charts:

```sh
curl -X POST localhost:3200/api/sessions -H 'Content-Type: application/json' \
  -d '{"date": "2026-06-01", "minutes": 95, "chars": 21400, "source": "other", "note": "import"}'
```

## Config

- `JP_TOOLS_STATS_DB_PATH` (default `~/.local/share/jp-stats/stats.db`) — must
  match what `vn-ws-logger.py` uses (same env var).
- `JP_TOOLS_STATS_LISTEN_ADDR` (default `0.0.0.0:3200`)
- `JP_TOOLS_ANKI_URL` (default `http://localhost:8765`) — fallback AnkiConnect
  when the dashboard client has none; `JP_TOOLS_ANKI_DECK` (`Japanese`),
  `JP_TOOLS_ANKI_FIELD_VOCAB` (`VocabKanji`)
- `JP_TOOLS_SUDACHI_DICT_PATH` (default `system_full.dic` in the working dir)
- `JP_TOOLS_VN_CAPTURE_SH` (default `../vn-mine/vn-capture.sh` relative to the
  crate) — what `#read`'s mine button runs. It needs the desktop session's
  environment (`spectacle` screenshots the active window), so read-stats has to
  be started from within the session, as `scripts/start-all.sh` does.

## Extending to new sources

Any reader with a line stream can insert into `lines` with its own `source`
tag (same schema, WAL mode — concurrent writers are fine); anything
session-shaped POSTs to `/api/sessions`. Derivation and the dashboard pick
both up without changes.
