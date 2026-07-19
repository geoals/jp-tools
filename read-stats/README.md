# read-stats

Automatic daily reading tracker: characters read and active reading time,
derived from the raw line stream `vn-mine/vn-ws-logger.py` already captures —
no manual copying, no counters to reset. Dashboard with goal meter (floor /
target minutes), streak, daily-minutes chart, and a chars/hour trend.

## How it works

- **Ingestion is passive.** `vn-ws-logger.py` (running under the `vn-buffer`
  systemd unit) inserts every hooked line — timestamp, whitespace-stripped
  char count, text — into `~/.local/share/jp-stats/stats.db`. The web service
  only reads that DB, so stats are captured whenever you read, whether or not
  the dashboard is running.
- **Everything is derived at query time** from raw line events, so thresholds
  are tunable after the fact:
  - a gap between lines credits reading time, capped at `afk_secs` (20,
    matching a texthooker-ui-style AFK timer);
  - a gap over `session_gap_secs` (600) closes the session;
  - days roll over at `day_rollover_hour` (04:00) — late-night reading counts
    toward the evening's day.
- **Pause** (`POST /api/pause` toggle, dashboard button, or bind
  `jp-stats-pause.sh` to a hotkey) for skipping scenes / replaying read text:
  lines are still captured raw but derivation ignores those inside a pause
  interval, so a forgotten pause can be fixed retroactively by editing the
  `pauses` table.
- **Manual sessions** cover everything without a line stream: physical books
  (pages × `chars_per_page`, default 550 ≈ bunkobon), manga, or imported
  history. Logged from the dashboard form or `POST /api/sessions`.

## Run

```sh
cargo run -p read-stats     # http://localhost:3200
```

Or as part of the stack: `scripts/start-all.sh`.

## API

- `GET  /api/summary` — today (chars, active seconds, per-source), goal, streak
- `GET  /api/days?days=60` — zero-filled per-day totals, oldest first
- `GET  /api/sessions?date=2026-07-19` — derived VN sessions + manual sessions
- `POST /api/sessions` — `{date?, start_ts?, minutes, chars? | pages?, work?, source?, note?}`
- `DELETE /api/sessions/{id}`
- `GET/PUT /api/settings` — `afk_secs`, `session_gap_secs`, `day_rollover_hour`,
  `goal_floor_mins`, `goal_target_mins`, `chars_per_page`

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

## Extending to new sources

Any reader with a line stream can insert into `lines` with its own `source`
tag (same schema, WAL mode — concurrent writers are fine); anything
session-shaped POSTs to `/api/sessions`. Derivation and the dashboard pick
both up without changes.
