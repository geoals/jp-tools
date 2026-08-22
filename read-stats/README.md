# read-stats

Automatic daily reading tracker: characters read and active reading time,
derived from the raw line stream `vn-mine/vn-ws-logger.py` already captures — no
manual copying, no counters to reset. Plus `#read`, the live line feed read
beside the running VN, where Yomitan does its lookups and mining.

`CLAUDE.md` has the architecture and the invariants; this file is the reference:
what the thing does, how to set it up, the endpoints and the config.

## How it works

- **Ingestion is passive.** `vn-ws-logger.py` (under the `kotodex-capture` systemd
  unit) inserts every hooked line — timestamp, char count, text — into the shared
  `knowledge.db`. Stats are captured whenever you read, whether or not the
  dashboard is running.
- **Characters are counted like texthooker-ui does** (`jp_core::text::chars`,
  mirrored in `vn-ws-logger.py`): an allowlist of kana, kanji, radicals and
  alphanumerics, so punctuation doesn't inflate chars/h. Startup recomputes
  `lines.chars` for any row that disagrees.
- **Everything is derived at query time**, so thresholds are tunable after the
  fact: a gap credits reading time up to `afk_secs` (30), a gap over
  `session_gap_secs` (600) closes the session, and days roll over at
  `day_rollover_hour` (04:00) so late-night reading counts to the evening.

  The 30 comes from the measured gap distribution rather than from feel: gaps
  containing a lookup cluster at 10–32s (median 24, p90 32) while gaps without
  one have a p90 of 9. At 20 the majority of lookups were being clipped,
  inflating chars/h by ~6%.
- **Yomitan lookups are counted by proxying AnkiConnect**
  (`routes/ankiproxy.rs`). Yomitan checks Anki for duplicates on every definition
  popup, so with its server address pointed at `/anki-proxy` each popup becomes a
  row in `lookups`. Requests are forwarded byte-for-byte, so mining is
  unaffected, and a lookup is recorded before forwarding, so it counts even with
  Anki closed. Repeats of the same term within 3s collapse into one. See
  *Counting lookups*.
- **Focus measures how continuous the reading was**, not how much of it there
  was (`stats::focus`): credited time hides fragmentation, so focus keeps the
  *uncapped* span beside it and reports `active / span`. Gaps over
  `session_gap_secs` are excluded (that is leaving, not being distracted), and a
  gap over 60s counts as an interruption only if nothing in it proves you were
  there. Manual sessions have no focus figure.
- **Pause capture** (`POST /api/capture/pause`, or the dashboard and `#read`
  buttons) stops the *source*: `settings.capture_paused` is polled by
  vn-ws-logger.py, which closes its Textractor WebSocket while it is set. Nothing
  is recorded and nothing can be recovered — which is why the reading view says
  so in red across the top.
- **Clear last line** (`✕ clear last` on `#read`) is the retroactive version: it
  flags the newest line `discarded` and every read filters it out. It covers the
  two things a pause is always remembered too late for — the junk Textractor
  hooks while you are still finding the route, and a stretch re-read after
  skipping back. One tap per line, and consecutive taps accumulate into one undo,
  offered on the toast for 15s. Nothing is deleted; a clear can be undone past
  the toast with `UPDATE lines SET discarded = 0 WHERE id = ?`.

  Clearing widens the gap around what it removed, which is the point: with the
  junk gone the surrounding span has no evidence in it, so the *time* stops being
  credited along with the characters.

  One caveat: `word_days` is **not** rewound, so if tokenization already ran over
  a cleared line its counts stay, very slightly inflating the re-encounter card.
  Everything else is derived fresh.
- **Manual sessions** cover everything without a line stream: physical books
  (pages × `chars_per_page`, default 550 ≈ bunkobon), manga, imported history.
  Logged from the dashboard form or `POST /api/sessions`.
- **Work metadata** turns per-work totals into progress. A work row (keyed by the
  title stamped on lines and sessions) carries a `total_chars` count pasted from
  the VN's jpdb page and optionally a cover: pass a VNDB id once and the art is
  fetched from `api.vndb.org/kana`, cached in `covers/` and served at `/covers/`.
  The currently-reading card shows progress, this VN's own speed, hours left and
  a projected finish date.

  The finish date is **this work's speed × your daily active hours**, the hours
  taken from the trailing 7 complete days (clipped to `pace_start_date`). Speed
  is a property of the VN and daily hours a property of you, so a fresh harder VN
  does not inherit an easier one's chars/day. Under 10 minutes in it falls back
  to the cross-work rate. A finished work shows its real dates instead.
- **Anki integration is read-only.** On dashboard load (or the ↻ button) the
  server probes for AnkiConnect — the client's own IP first, for a phone running
  AnkiconnectAndroid, then `JP_TOOLS_ANKI_URL` — and snapshots the deck's
  `VocabKanji` fields into `anki_notes`. Note ids double as creation timestamps,
  which gives cards-per-session for free. New lines are tokenized into per-day
  lemma counts (`word_days`), which power the **re-encounter card**: how many
  mined words the reading has since shown you again.

## The reading view (`/#read`)

A live feed of the lines Textractor hooks, read beside the VN while it runs — the
lines stay visible and selectable, so there is no reaching into the game window
for a lookup. Served over the LAN and Tailscale too, so a phone beside the screen
works the same way.

- **Yomitan** scans the lines. Point its *Server address* at `/anki-proxy` so
  lookups are counted and cards land in Anki.
- **Mining has no button.** Yomitan's `addNote` goes through `/anki-proxy`, and
  the proxy runs `vn-mine/vn-capture.sh` once Anki accepts the note, so audio and
  a screenshot attach to every mine. whisper-service is *optional* here — it only
  narrows the clip to the mined sentence within a multi-sentence line. When it is
  down the VAD-trimmed clip is attached instead and the bar shows a muted **✂
  off** hint.
- **✕ clear last** drops the newest hooked line from the stats.
- **ℹ explain last line** sends the newest line, with a few before it for
  context, to the Anthropic API and shows a short read on it. **Select a word
  first** and the explanation centres on that word; the selection is read the
  instant the button is tapped. Capped at a few sentences, on
  `claude-haiku-4-5` by default (`JP_TOOLS_LLM_MODEL`), and only enabled when
  `JP_TOOLS_ANTHROPIC_API_KEY` is set.
- **Tapping a word judges it** — see CLAUDE.md for the rules.

While the reader is open the **page title is set to `current_work`**, because
Yomitan's `{document-title}` marker fills the note's Document field — so the tab
title is what a mined card records as its source. Re-read every 20s. If no work
is set, cards get stamped "read-stats"; set the work first.

Cards must land in the collection on the machine running the VN, since that is
what `vn-capture.sh` attaches media to, so the proxy forwards to
`JP_TOOLS_ANKI_URL` unconditionally rather than preferring the requesting client
the way manga-mine's export does. The 5-minute ring-buffer limit applies either
way: mine before advancing.

The feed reads the `lines` table rather than opening a second Textractor
WebSocket, whose plugin can crash the game on an abortive client disconnect.

## The dashboard

Five tabs, one per question, all fed by one poll — the tabs choose what renders,
never what is fetched, so two of them cannot disagree about a day.

- **Today** — what you are reading and how far in, then the day itself: goal
  meter, totals, intra-day curve and sittings, all following one date.
- **Trends** — one range (7/30/60d) over the summary tiles, daily bars, the speed
  panel and the lookup/card-rate panel.
- **Library** — the works (shelf → per-work page), the vocabulary funnel, and the
  manual log form.
- **Kanji** — every kanji ever read, tinted by encounter count, with grade
  coverage and a discovery curve.
- **Vocab** — the ledger's status counts and the triage sweep that fills them.

## Settings (`/#settings`)

Two kinds of thing, and the split is deliberate:

- **server settings** — rows in `settings`, applied at *query* time: the goal
  (daily target, streak minimum) and the derivation thresholds (gap cap, session
  break, day rollover, chars per page). Nothing is baked into a stored number, so
  changing one re-reads the whole history under the new value — raise the gap cap
  and every hour you have ever read is re-priced, lower it and they go back.
- **this browser** — the theme, in `localStorage` and applied as `data-theme` on
  `<html>`. Stamped by a blocking script in `spa.html` before first paint, so a
  dark device doesn't flash light on load.

The current work and the VN capture window are deliberately *not* here: both are
per-work workflow rather than configuration, and they live beside the work they
describe.

## Run

```sh
cargo run -p read-stats     # http://localhost:3200
```

Or as part of the stack: `scripts/start-all.sh`, which takes service names
(`start-all.sh restart read-stats`). The `kotodex-capture` ingestion daemon is a
separate systemd user unit: `systemctl --user start kotodex-capture`.

## API

- `GET  /api/summary` — today (chars, active seconds, per-source, lookups),
  goal, streak
- `GET  /api/days?days=60` — zero-filled per-day totals, oldest first; each day
  also carries `work`, the title that read the most characters that day, so the
  speed chart can mark where reading switched VNs
- `POST /anki-proxy` — AnkiConnect pass-through that counts Yomitan lookups;
  point Yomitan's server address here (see *Counting lookups*)
- `GET  /api/day/timeline?date=2026-07-19&bucket_secs=60` — one day sliced into
  fine buckets (`{t, session, chars, active_secs, lookup_secs, lookups, cards}`)
  plus the day's session spans. Smoothing is deliberately *not* done here: the
  buckets are finer than anything worth plotting and the client rolls them up,
  so the dashboard's granularity slider never re-queries. See *Day detail*
- `GET  /api/sessions?date=2026-07-19` — derived VN sessions + manual sessions
- `POST /api/sessions` — `{date?, start_ts?, minutes, chars? | pages?, work?, source?, note?}`
- `DELETE /api/sessions/{id}`
- `GET  /api/works` — per-work (title) totals, merging line-stream and manual
  sessions, each with its metadata (`meta`: total_chars, cover, status,
  `vn_window`); metadata-only works (e.g. queued) get a zero row
- `POST /api/works` — create/update metadata by title:
  `{title, vndb_id?, total_chars?, status?, queue_pos?, vn_window?}` — `vndb_id`
  accepts `v3144` / `3144` / a vndb.org URL, is used once to fetch the cover and
  not stored; empty string removes the cover; `total_chars: 0` clears; status ∈
  reading/queued/finished/dropped; `vn_window` is the capture-target substring
  for this VN (empty string clears)
- `PUT  /api/works/{id}` / `DELETE /api/works/{id}` — same fields by id / remove
- `GET  /api/lines/stream` — SSE, one event per hooked line, `data` being
  `{id, ts, chars, text}` and the event id being the line id. Opens on the
  sitting in progress, widened to 200 lines when that sitting is shorter than
  that, so a feed opened onto a session that has just started still has
  something to look back over; `?backlog=<n>` asks for a fixed tail instead, and
  `?after=<id>` / `Last-Event-ID` resumes so a reconnecting client doesn't
  replay or skip
- `POST /api/lines/discard` — `{ids: [...]}` (max 500), flags those lines
  `discarded` so every derived figure drops them; returns the ids actually
  changed, which is what undo re-sends. `POST /api/lines/undiscard` is the
  inverse. See *How it works* → clear last line
- `GET  /api/reader/state` — `{paused, current_work, capture_available,
  explain_available, trim_available}`. `trim_available` is a live probe of
  whisper-service (`JP_TOOLS_WHISPER_URL`, 800 ms timeout) — false lights the
  reader's **✂ off** hint; capture doesn't depend on it
- `POST /api/reader/explain` — `{context: [oldest…newest], focus?}`; sends the
  lines to the Anthropic API and returns `{text}`, a short explanation of the
  last one centred on `focus` if given. 400 if no key is configured or the
  context is empty; the context is capped server-side. See *The reading view*
- `GET  /api/vn/windows` — open window titles (via xdotool, Wine/Qt/IME
  scaffolding filtered out), offered as a picker for a work's `vn_window`
- `POST /api/vn/capture` — run `vn-capture.sh` (see `JP_TOOLS_VN_CAPTURE_SH`)
  and return its result. A capture that fails for an ordinary reason (stale
  line, Anki closed) is `200 {"ok": false, "error": ...}`; only an unrunnable
  or unparseable script is a 5xx
- `POST /api/capture/pause` — toggle capture at the source (`settings.capture_paused`)
- `POST /api/anki/refresh` — probe AnkiConnect (client IP, then fallback),
  snapshot the deck, tokenize new lines
- `GET  /api/anki/summary` — mined count, re-encountered count, 7-day
  encounters, top words, never-re-encountered sample
- `GET  /api/lookups/summary` — lookup outcomes per distinct term (mined /
  already-carded / never carded), repeat-lookup list, leech list, median
  lookup→card latency
- `GET/PUT /api/settings` — `afk_secs`, `session_gap_secs`, `day_rollover_hour`,
  `goal_floor_mins`, `goal_target_mins`, `chars_per_page`, `current_work`,
  `vn_window` (legacy global fallback; the VN window is now a per-work column —
  see `PUT /api/works/:id` — so it travels with the VN instead of going stale on
  a switch),
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
sqlite3 ~/.local/share/jp-tools/knowledge.db 'SELECT ts, term, work FROM lookups ORDER BY id DESC LIMIT 5;'
```

An empty table with popups appearing means the request shape wasn't recognized —
run the server with `RUST_LOG=read_stats=debug` and look for
`lookup action with no extractable term`.

### What lookups turn into

The *Lookups* card classifies each distinct looked-up term by comparing the
card's creation time (an Anki note id is epoch ms) against the term's first
lookup:

- **became cards** — a card was made at or after the lookup; the lookup stuck.
- **already had a card** — a card existed *before* the lookup: a word that was
  mined but didn't take. These are leeches, listed with the card's age.
- **repeat lookups** — the same word looked up more than once, worst first, each
  tagged with its outcome. An unmined repeat is a mining candidate; a carded
  repeat is a card that isn't working.

Counts are over distinct terms, not lookup events, so a word looked up five times
before being mined counts once. All of it joins `lookups.term` to
`anki_notes.vocab`, and `anki_notes` is a **snapshot** — refresh first, or
anything mined since the last one reads as "never carded".

`lookups/1k` is lookups per 1000 characters: the unknown-word rate, suppressed
below 500 chars/day where the ratio is mostly noise. The *Lookups & cards* chart
plots lookups/h against cards/h — both events per hour, so they share a y-axis.
Days under 10 minutes are omitted. Minutes read stays in its own chart, since a
second y-scale would imply a correlation the data doesn't contain.

### Day detail

One day down to the minute: reading speed on top, lookups/h and cards/h below, on
a shared clock axis. A slider sets the smoothing window (1–45 min) and a date
picker walks back through history.

**The speed panel carries two lines.** *As read* is
`(clean_chars + lookup_chars) / active_secs` — what actually happened. *Lookups
removed* is `clean_chars / (active_secs − lookup_secs)`: speed over the gaps that
held no lookup. Both are rates over characters that have seconds attributed to
them, which is why the numerator isn't plain `chars`. The shaded gap between them
is the **lookup tax**, with the whole-day figure stated below the chart.

A gap counts as lookup time when a `lookups` row falls inside it — a separation
sharp enough to trust, at a median 21.3s against 3.1s for gaps without one. It is
all-or-nothing per gap, which biases the tax upward: a long gap catches a lookup
and is billed whole even when the dictionary wasn't why it was long. Good to a
couple of points, not to the decimal.

**Time lost to lookups is not the same as time inside lookup gaps.** Such a gap
holds the line's reading *and* the detour, so the note under the chart prices the
characters in those gaps at the window's uninterrupted pace and subtracts.

The two panels are stacked rather than overlaid because chars/hour runs in the
thousands and events/hour in the tens, and where two y-scales line up is a
choice, not a fact. The **⇕ overlay shape** toggle does draw the rate curves into
the speed panel for timing comparison, each normalised to its own max — which
makes co-movement obvious and amplitude meaningless, so magnitude stays with the
lower panel and the tooltip.

Bucketing places a gap's credit in the interval *after* its line
(`[ts, ts + min(gap, afk)]`) rather than in the following line's bucket, so a
line's characters and the seconds they cost land together. At day granularity
that is invisible; at one minute it is the difference between a curve and noise.
Totals are unaffected (`bucket_totals_match_session_totals`).

Lookups and cards outside every session are dropped from the buckets — with no
reading time around them there is no per-hour rate they belong to — so the card's
event counts can sit a little under the `/api/days` totals.

### What counts as being there

A gap inside `afk_secs` is credited whole — ordinary reading, none of it in
doubt. Past the cap the question is whether you were still at the keyboard, and
the answer comes from evidence rather than a flat rate:

- **A lookup, a mined card, or a `#read` engagement action in the gap** proves
  you were present when it fired, so the clock restarts there and runs a fresh
  `afk_secs`. Reading a definition happens *after* the event, so a 45-second
  detour is credited 45, not truncated to 30.

  The engagement action is the **ℹ explain** button, recorded as a `reader_mark`.
  It fills the one gap the other two leave: reading an explanation is real
  presence the line stream has no other trace of. Kept in its own table so it
  credits *time* without touching the lookup rates. The *suppress* actions —
  clear and pause — deliberately leave no mark, since crediting presence for them
  would undo their own purpose.
- **Nothing in the gap** means only the line itself is claimed, priced at your
  uninterrupted pace. A 15-character line earns about four seconds whether you
  were gone 35 seconds or seven minutes.

The upshot is that you never have to think about the afk timer: walking away
costs nothing and is never credited, so pausing capture is about keeping junk out
of the stream, not about protecting the numbers.

### Importing spreadsheet history

One manual session per historical day carries old totals into streaks and
charts:

```sh
curl -X POST localhost:3200/api/sessions -H 'Content-Type: application/json' \
  -d '{"date": "2026-06-01", "minutes": 95, "chars": 21400, "source": "other", "note": "import"}'
```

## Config

- `JP_TOOLS_KNOWLEDGE_DB_PATH` (default `~/.local/share/jp-tools/knowledge.db`) —
  the shared database holding the line stream; must match what
  `vn-ws-logger.py` writes to
- `JP_TOOLS_STATS_DB_PATH` (default `~/.local/share/jp-tools/read-stats.db`) — must
  match what `vn-ws-logger.py` uses (same env var).
- `JP_TOOLS_STATS_LISTEN_ADDR` (default `0.0.0.0:3200`)
- `JP_TOOLS_ANKI_URL` (default `http://localhost:8765`) — fallback AnkiConnect
  when the dashboard client has none; `JP_TOOLS_ANKI_DECK` (`Japanese`),
  `JP_TOOLS_ANKI_FIELD_VOCAB` (`VocabKanji`)
- `JP_TOOLS_ANKI_FIELD_SENTENCE` (`SentKanji`), `JP_TOOLS_ANKI_FIELD_COMPACT_DEF`
  (`CompactDef`) — when a card is added through `/anki-proxy` (Yomitan mining a
  VN line), the proxy forwards it unchanged and then, in the background,
  generates a ≤2-second CompactDef gloss from the note's word + sentence and
  writes it to that field. Needs `JP_TOOLS_ANTHROPIC_API_KEY`; set the field
  name empty to disable.
- `JP_TOOLS_AUTO_CAPTURE_ON_ADD` (default **on**) — fire `vn-capture.sh` after a
  proxied card add (audio + picture, best-effort). This *is* mining now; there
  is no button. Set to `0` on a machine that serves the dashboard but doesn't
  run the VN — where the capture script is simply absent it already no-ops with
  a warning.
- `JP_TOOLS_SUDACHI_DICT_PATH` (default `system_full.dic` in the working dir)
- `JP_TOOLS_VN_CAPTURE_SH` (default `../vn-mine/vn-capture.sh` relative to the
  crate) — what the proxy runs after a card add. It needs the desktop session's
  environment (`spectacle` screenshots the active window), so read-stats has to
  be started from within the session, as `scripts/start-all.sh` does.
- `JP_TOOLS_ANTHROPIC_API_KEY` — enables `#read`'s ℹ explain last line button; unset
  leaves it disabled. `JP_TOOLS_LLM_MODEL` (default `claude-haiku-4-5`) — the
  model it asks. Both are shared with yt-mine, so a root `.env` covers both.
- `JP_TOOLS_WHISPER_URL` (default `http://localhost:8100`) — whisper-service,
  probed only to light the reader's **✂ off** hint. read-stats never calls it
  directly; `vn-capture.sh` does (its own `VN_WHISPER_URL`), and the mine works
  whether or not it's up.

## Extending to new sources

Any reader with a line stream can insert into `lines` with its own `source`
tag (same schema, WAL mode — concurrent writers are fine); anything
session-shaped POSTs to `/api/sessions`. Derivation and the dashboard pick
both up without changes.
