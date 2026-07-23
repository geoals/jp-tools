# read-stats

Automatic daily reading tracker: characters read and active reading time,
derived from the raw line stream `vn-mine/vn-ws-logger.py` already captures —
no manual copying, no counters to reset. Dashboard with goal meter (floor /
target minutes), streak, a daily bar chart switchable between minutes and
characters and stackable by dialogue, a chars/hour trend, a
toggleable lookups/h vs cards/h trend, a minute-resolution *Day detail*
view that prices the lookup tax against reading speed, and a *Dialogue vs
narration* card that splits the reading on 「」.

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
- **Dialogue is separated from narration by bracket depth** (`dialogue.rs`).
  Japanese marks speech with 「…」 (and 『…』 nested or for titles), so the
  distinction is already in the hooked text and costs nothing to derive. The
  split is per *character*, not per line, so `「そうか」と彼は言った` is three
  dialogue characters and six narration; that makes it an exact partition of
  `count_chars`, which is what lets the share be quoted against the same
  totals everything else uses. Only corner brackets count — “…” is used for
  emphasis and for quoting a phrase rather than a speaker, and （…） is usually
  inner monologue.

  Bracket depth **carries across lines**: a long speech is hooked as several
  text boxes, so its 「 opens on one row and closes on a later one. Resetting
  per line would file every continuation as narration. It is dropped across a
  gap over `dialogue::CARRY_GAP_SECS` (300), which is far past any real
  continuation, so one dropped 」 can't recolor a whole session as speech.

  See *Dialogue vs narration* below for what the numbers say.
- **Yomitan lookups are counted by proxying AnkiConnect** (`ankiproxy.rs`).
  Yomitan checks Anki for duplicates every time it shows a definition popup, so
  with its server address pointed at `/anki-proxy` each popup becomes a row in
  `lookups`. Requests are forwarded to the real AnkiConnect byte-for-byte, so
  mining is unaffected; a lookup is recorded before forwarding, so it still
  counts when Anki is closed. Repeated requests for the same term within 3s
  collapse into one lookup (a single popup fires several). See *Counting
  lookups* below.
- **Focus measures how continuous the reading was**, not how much of it there
  was (`stats.rs::aggregate_focus_days`). Credited time hides fragmentation by
  design, so focus keeps the *uncapped* span beside it and reports
  `active / span`. 100% means every gap was reading; 60% means two fifths of
  your at-desk time went somewhere else. Gaps over `session_gap_secs` are
  excluded (that's leaving, not being distracted); a gap over 60s counts as an
  interruption and breaks the longest-stretch run **only if nothing in it proves
  you were there**. Needs the line stream, so manually logged sessions have no
  focus figure.

  Both halves of that have to use the same `Presence` rule the rest of the app
  does, and for a while they didn't. On 2026-07-20 focus read 97.3% with a
  52-minute longest stretch; all 17 gaps behind the missing 2.7% held lookups,
  and the one "interruption" splitting the stretch was a 93-second sentence
  worked through with four of them. Corrected: **99.6%, one unbroken stretch of
  98.5 minutes, zero interruptions** — which is what the reading actually was.
  A metric that counts using a dictionary as losing focus is measuring the
  wrong thing.
- **Pause** (`POST /api/pause` toggle, dashboard button, or bind
  `jp-stats-pause.sh` to a hotkey) for skipping scenes / replaying read text:
  lines are still captured raw but derivation ignores those inside a pause
  interval, so a forgotten pause can be fixed retroactively by editing the
  `pauses` table.
- **Clear last line** (`✕ clear last` on `#read`) is the retroactive version of
  pause: it flags the newest line `discarded`, and every read of the stream
  filters that out. It covers the two things pause is always remembered too
  late for — the handful of lines Textractor hooks while you are still finding
  the route, which are otherwise enough to open a session, and a stretch
  re-read after skipping back, which would otherwise be counted a second time.

  One tap per line, and the line leaves the feed as it goes, so tapping until
  the junk is gone needs no count in the UI. The ids come from what is on
  screen rather than the server picking "the last one", so a line hooked
  between the tap and the request isn't swept up with them. Consecutive taps
  accumulate into one undo, offered on the toast for 15s.

  Nothing is deleted — the flag is soft, for the same reason pauses don't
  delete: the raw stream is what lets every threshold here stay tunable after
  the fact. A clear can be undone past the toast with
  `UPDATE lines SET discarded = 0 WHERE id = ?`.

  Clearing widens the gap around what it removed, and that is the point: with
  the junk lines gone the surrounding span has no evidence in it, so the
  *time* stops being credited along with the characters. For a re-read stretch
  that means the minutes spent skimming already-read text stop counting too,
  which is the right call — it was re-reading, not reading.

  One caveat: `word_days` is **not** rewound. If tokenization already ran over
  a cleared line (it runs on Anki refresh, so usually it hasn't) its lemma
  counts stay, very slightly inflating the re-encounter card. Everything else —
  chars, time, speed, focus, the dialogue split — is derived fresh and correct.
- **Manual sessions** cover everything without a line stream: physical books
  (pages × `chars_per_page`, default 550 ≈ bunkobon), manga, or imported
  history. Logged from the dashboard form or `POST /api/sessions`.
- **Work metadata** turns per-work totals into progress. A work row (keyed by
  the exact title stamped on lines/sessions) carries a `total_chars` count
  pasted manually from the VN's jpdb page (jpdb has no public API) and
  optionally a cover: pass a VNDB id once and the cover is fetched from
  `api.vndb.org/kana`, cached next to the DB in `covers/`, served at
  `/covers/` — nothing else from VNDB is stored. The currently-reading card
  shows cover, char-based progress bar, this VN's own reading speed, hours left
  at that speed, and a projected finish date. The finish date is decomposed:
  **this work's speed × your daily active hours**, the hours taken from the
  trailing 7 complete days (clipped to `pace_start_date`, for coming back from a
  break). Speed is a property of the VN and daily hours a property of you, so a
  fresh harder VN no longer inherits an easier one's chars/day. Under 10 minutes
  into a work it falls back to the cross-work chars/day until it can gauge the
  work's own speed. A **finished** work shows its real started/finished dates
  instead of a projection. The Library lists every work's own chars/h so they
  compare directly, and the speed chart marks where reading switched VNs.

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
- **✕ clear last** drops the newest hooked line from the stats — see *Clear
  last line* above. Deliberately narrow and quiet next to the mine button,
  which is the one being pressed constantly; every press is undoable.
- **⛏ mine last line** runs `vn-mine/vn-capture.sh` on the PC, attaching the
  voiceline audio and a screenshot to the note Yomitan just added, and reports
  the outcome on the page instead of via `notify-send` on a desktop nobody is
  looking at. **whisper-service is optional here:** it only narrows the clip to
  the single mined sentence within a multi-sentence line. When it's down the
  mine still works — the clip is attached VAD-trimmed — and the reader bar shows
  a muted **✂ off** hint (from `trim_available` in `/api/reader/state`, probed
  each poll) so you know the sentence trim isn't running.
- **ℹ explain** sends the newest line (with the previous few for context) to the
  Anthropic API and shows a short read on it — a natural rendering plus any
  nuance or grammar a plain translation misses. **Select a word in the line
  first** and the explanation is centred on that word instead; the selection is
  read the instant the button is tapped, so opening the panel doesn't clear it.
  For a light lookup while reading, not a full translation — the reply is capped
  at a few sentences and the model defaults to `claude-haiku-4-5`
  (`JP_TOOLS_LLM_MODEL`). The button is only shown enabled when
  `JP_TOOLS_ANTHROPIC_API_KEY` is set. The panel stays up (scrolling internally)
  until dismissed, so it can sit open while looking back over the line.

While the reader is open the **page title is set to `current_work`**, not
"read-stats". Yomitan's `{document-title}` marker is what fills the note's
Document field, so the tab title is what a mined card records as its source —
it has to be the VN. `current_work` is re-read every 20s, so switching works on
the dashboard takes effect without reloading the reader. If no work is set the
title is left alone, and cards will be stamped "read-stats" — set the work
first.

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

Or as part of the stack: `scripts/start-all.sh`. To run only the VN reading
stack (read-stats + optional whisper-service, skipping yt-mine and manga):
`scripts/vn.sh` — see its `--help`. The passive `vn-buffer` ingestion daemon is
a systemd user unit (`systemctl --user start vn-buffer`); `vn.sh status` reports
it but doesn't manage it.

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
  `{id, ts, chars, text}` and the event id being the line id. Sends the last
  `?backlog=` lines (40) on open, or resumes after `?after=<id>` /
  `Last-Event-ID` so a reconnecting phone doesn't replay or skip
- `POST /api/lines/discard` — `{ids: [...]}` (max 500), flags those lines
  `discarded` so every derived figure drops them; returns the ids actually
  changed, which is what undo re-sends. `POST /api/lines/undiscard` is the
  inverse. See *Clear last line*
- `GET  /api/reader/state` — `{paused, current_work, capture_available,
  explain_available, trim_available}`. `trim_available` is a live probe of
  whisper-service (`JP_TOOLS_WHISPER_URL`, 800 ms timeout) — false lights the
  reader's **✂ off** hint; capture doesn't depend on it
- `POST /api/reader/explain` — `{context: [oldest…newest], focus?}`; sends the
  lines to the Anthropic API and returns `{text}`, a short explanation of the
  last one centred on `focus` if given. 400 if no key is configured or the
  context is empty; the context is capped server-side. See *Reading from a
  phone*
- `GET  /api/vn/windows` — open window titles (via xdotool, Wine/Qt/IME
  scaffolding filtered out), offered as a picker for a work's `vn_window`
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
- `GET  /api/dialogue/summary?days=60&work=<title>` — the 「」 split: `today` and
  per-`day` character shares, plus `overall` with speed, clean speed and
  lookups/1k for each side. `work` scopes it to one VN (its lines only, filtered
  before aggregation); absent/empty pools all works. See *Dialogue vs narration*
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

### Day detail

The *Day detail* card zooms one day down to the minute: reading speed on top,
lookups/h and cards/h below, on a shared clock axis. A slider sets the smoothing
window (1–45 min) and a date picker walks back through history.

**The speed panel carries two lines.** *As read* is `(clean_chars +
lookup_chars) / active_secs` — what actually happened. *Lookups removed* is
`clean_chars / (active_secs − lookup_secs)`: reading speed over the gaps that
contained no lookup. Both are rates over characters that have seconds
attributed to them, which is why the numerator isn't plain `chars`: a session's
trailing line has no gap after it and so cost no credited time, and leaving it
in one side of the comparison only would understate the tax. The shaded
gap between them is the **lookup tax**, read straight off the chars/hour axis,
with the whole-day figure stated in words below the chart.

**Both sides of that ratio have to drop together.** A line's characters were
read across the gap that follows it, so when that gap held a lookup the
characters leave the numerator along with their seconds (`clean_chars` exists
for exactly this). Dividing *all* chars by only the non-lookup seconds instead
credits characters read during a lookup to the time that remains — and in a
dense lookup burst the denominator collapses while the numerator doesn't. That
bug reported 30k chars/h for reading that was really running at 12k;
`raw_speed_cannot_explode_in_a_lookup_burst` pins it.

A gap counts as lookup time when a `lookups` row falls inside it. The
separation is sharp enough to trust: over 2026-07-20's 1220 in-session gaps,
those holding a lookup ran a median 21.3s against 3.1s for those that didn't.

The classification is nonetheless all-or-nothing per gap, which **biases the
tax upward**: a gap is long for reasons other than the dictionary too — a
stretch, a re-read, a stray thought — and the longer it runs, the likelier it
is to also catch a lookup and be billed whole. Two things bound how much of the
20% this could be. The `lookup_chars` subtraction above already removes the
reading inside those gaps, and an 18-second spread between the two medians is
far more than idle-catching alone would produce at 7% of gaps. Treat the figure
as good to a couple of points, not to the decimal.

**Time lost to lookups is not the same as time inside lookup gaps.** Such a gap
holds the line's reading *and* the dictionary detour, so the note under the
chart prices the characters in those gaps (`lookup_chars`) at the window's
uninterrupted pace and subtracts. On 2026-07-20: 30.8 min sat in lookup gaps,
9.2 min of it was reading that would have happened anyway, leaving **21.5 min
of real lookup overhead** — a median 14.1s per gap, at 1.3 lookups per gap. The
chars/h tax is unaffected by this correction, being a ratio of rates that
already accounts for characters read during lookups.

### What counts as being there

A gap inside `afk_secs` is credited whole — that is ordinary reading and none of
it is in doubt. Past the cap, the question is whether you were still at the
keyboard, and the answer comes from evidence rather than a flat rate:

- **A lookup, a mined card, or a #read engagement action in the gap** proves you
  were present when it fired, so the clock restarts there and runs a fresh
  `afk_secs`. A lookup is not instantaneous — reading the definition happens
  *after* the event — so a 45-second detour is credited 45, not truncated to 30
  the way the old flat cap did it.

  The engagement actions are the reader's **ℹ explain** and **⛏ mine** buttons,
  recorded as `reader_marks` when tapped (see *Reading from a phone*). They fill
  the one gap the other two signals leave: reading an explanation, or mining a
  line you didn't also look up in Yomitan, is real presence the line stream has
  no other trace of. Kept in their own table, not `lookups`, so they credit
  *time* without touching the lookups/h or unknown-word-rate metrics. The
  *suppress* actions — **clear** and **pause** — deliberately leave no mark: they
  exist to stop counting a span, so crediting presence for them would undo their
  own purpose.
- **Nothing in the gap** means only the line itself can be claimed, priced at
  your uninterrupted pace. A 15-character line earns about four seconds whether
  you were gone 35 seconds or seven minutes.

The flat cap this replaced paid a blanket 30 seconds into *every* over-cap gap.
On 2026-07-19 that was 44 absences, 22 minutes of reading that never happened,
and an 11% understatement of that day's speed. The rule cuts both ways: 07-19
loses 12 minutes, but 07-20 — a lookup-heavy day with barely any absence —
*gains* 3, because the extension credits detours the cap used to shear off. The
two days' speeds converge from 12,121/13,467 to 13,028/13,160, which is the
point: the metric should measure reading, not how often you remembered to hit
pause.

Pace comes from `stats::measure_pace` over **all history**, never the slice a
request happened to fetch. It is a property of the reader, and deriving it
per-endpoint had the dashboard measuring it across every day and the timeline
across one, so the same day's active minutes differed depending on which page
you opened.

Two guards worth knowing about. Sub-cap gaps are never repriced
(`ordinary_gaps_are_never_repriced`) — pricing each gap at what its line was
"worth" would clip every above-average gap to average and leave the
below-average ones, shortening a day by a quarter while calling it a
correction. And a stream too sparse to establish a pace falls back to the flat
cap, which is the right thing to degrade to.

You should not have to think about the afk timer, and with this you don't:
walking away costs nothing and is never credited, so the pause button is now a
convenience rather than something the numbers depend on.

Two panels rather than one overlay, because chars/hour runs in the thousands and
events/hour in the tens: one plot would need two y-scales, and where two scales
line up is a choice, not a fact. Stacked on a shared x-axis with a shared
crosshair, a speed dip and a lookup spike land in the same vertical slice and
the comparison stays the reader's. The **⇕ overlay shape** toggle does draw the
rate curves into the speed panel for timing comparison, each normalised 0→its
own max by a fixed rule — which makes co-movement obvious and amplitude
meaningless, so magnitude stays with the lower panel and the tooltip, both of
which report real per-hour values.

Bucketing places time differently from the per-day aggregates, on purpose. A
gap's credit goes to the interval *after* its line (`[ts, ts + min(gap, afk)]`)
rather than to the following line's bucket, because the gap after a line is the
time spent reading that line — that is what puts a line's characters and the
seconds they cost in the same bucket. At day granularity the difference is
invisible; at one minute it is the difference between a speed curve and noise.
Totals are unaffected either way (`bucket_totals_match_session_totals`).

Lookups and cards falling outside every session are dropped from the buckets —
with no reading time around them there is no per-hour rate they belong to — so
the card's event counts can sit a little under the day totals on
`/api/days`.

### Dialogue vs narration

The card answers three questions off one classification: what share of the
reading was people talking, whether speech and prose read at different speeds,
and which of the two carries more unknown words.

A **scope toggle** (all works / current work) sits in the header. All-works
pools your entire history; scoping to a VN filters to its lines before the split
is aggregated, so the numbers are that VN's own — and they diverge sharply (on
the current corpus 素晴らしき日々 is 69% dialogue at 14.5k/h, ドーナドーナ is 90%
dialogue at 8.8k/h; pooled sits between and belongs to neither). Per-VN scoping
relies on the same session-level line stream the rest of the app does, so
coarse interleaving is exact and only line-by-line alternation inside one
sitting would blur it.

On 素晴らしき日々 the answer is lopsided, and consistently so:

| | share of chars | chars/hour | lookups per 1k |
|---|---|---|---|
| dialogue | 70% | 14,300 | 2.0 |
| narration | 30% | 11,100 | 4.2 |

Narration reads about a fifth slower and needs **twice** the lookups per
character. That is the useful finding: the prose, not the speech, is where the
difficulty sits — so a work's difficulty is better predicted by how
narration-heavy it is than by its overall lookup rate, and a slow day may just
have been a description-heavy scene rather than a bad one.

**The share and the speeds are counted over different lines, deliberately.**

- *Share* is over every classified character, mixed lines included. It is
  asking what the text consisted of, so it cannot skip any of it.
- *Speed* and *lookups/1k* are over lines that were **wholly** one kind
  (`Side::timed_chars`). A gap is one undivided span of time: whatever was
  spent on `「そうか」と彼は言った` cannot be split into a dialogue part and a
  narration part afterwards, and charging it whole to the majority side would
  bias the comparison by exactly the quantity being compared. Mixed lines are
  dropped from both halves instead — 30 lines in 5183 on the current corpus.

Never divide one against the other; they are different populations.

**The daily bar chart takes the same split**, over two independent switches:
minutes or characters, stacked by dialogue or not. Minutes and characters share
one chart with a switch rather than getting two, because they are the same
question in the two units it has — and they are never overlaid, which would
need a second y-scale. Reading speed *is* the relationship between them and
already has its own chart.

The stack has a third segment, **no line text**, and it is a real category
rather than a rounding bucket: manually logged sessions (physical books, or
imported history) have no hooked text to classify, so those days are legitimately
all remainder. It is drawn in muted ink rather than a fourth hue because it is
the absence of the measurement, not a third kind of reading. The parts are never
rescaled to fill the bar — a bar's height is the day's real total in both modes,
so toggling the split changes what a bar is made of and never how tall it is.

Both speeds also come in a **lookups-removed** form (`clean_speed`), computed
the same way the day timeline does it: the characters read across a lookup gap
leave the numerator along with the seconds they cost. Dividing all of a side's
characters by only its uninterrupted seconds is the same unbounded inflation
`raw_speed_cannot_explode_in_a_lookup_burst` pins for the timeline.

Days with no line stream — manual sessions, or history imported before text was
kept — report a `null` share rather than 0%. "No text to split" is not "all
narration", and counting those rows would have dragged every old day's share
toward prose.

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
- `JP_TOOLS_ANTHROPIC_API_KEY` — enables `#read`'s ℹ explain button; unset
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
