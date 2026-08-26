# textractor — the Textractor source

Connects to Textractor's WebSocket plugin and posts what it hooks to the ledger.

- `vn-ws-logger.py` — connects to the Textractor WebSocket server
  (`ws://localhost:6677`, override with `VN_WS_URL`) and appends each hooked
  Japanese line to `lines.log` with a timestamp. Auto-reconnects if Textractor
  restarts. Also posts each line to the ledger's ingest endpoint
  (`POST /api/lines` on `http://127.0.0.1:3200`, override with
  `KOTODEX_SERVER_URL`) so reading time/chars are tracked automatically —
  best-effort, never blocks mining; disable with `KOTODEX_INGEST_DISABLE=1`.

  **It never opens a database.** It is one source among several, and it owns
  only what is specific to Textractor: the hooker's junk, a continuation split
  across two text boxes, the dedup. The character count, the work stamped on
  the line and whether capture is paused at all are the ledger's answers — so a
  second source cannot arrive at a different number for the same reading, and
  a source on a phone can do the same job. Started before the server it logs to
  `lines.log` alone and retries every 30s, so a first boot loses nothing.

  **Restarting the logger with Textractor attached is safe** as long as it goes
  through SIGTERM: `run()` sends a close frame before exiting, and the
  capture-pause path reuses the same `ws.close()`. What the WS plugin cannot
  survive is an **abortive** disconnect (`kill -9`, or a crash that skips the
  close frame) — so don't hard-kill it, and don't drop the signal handler.

- `test_ws_logger.py` — the logger's tests, which are the only check on the
  cleaning, the ruby split and the dedup. `pytest` is a development dependency
  and deliberately not
  in `requirements.txt`, so it is not in the venv `setup.sh` builds:

  ```sh
  pip install --user pytest
  ~/.local/share/kotodex/venv/bin/python -m pytest sources
  ```

  The venv's interpreter, because the module imports `websockets`.

## Running it by hand

Normally `capture/kotodex-capture` starts this, because a mined card needs the
line's timestamp and the audio taken by the same process. Nothing stops it
running alone, on a machine with no capture daemon and no audio:

```sh
KOTODEX_SERVER_URL=http://192.168.1.20:3200 \
  ~/.local/share/kotodex/venv/bin/python sources/textractor/vn-ws-logger.py
```

## Env

- `VN_WS_URL` (default from `settings.line_source_ws_url`, else
  `ws://localhost:6677`) — the Textractor WebSocket to hook.
- `KOTODEX_SERVER_URL` (default `http://127.0.0.1:3200`) — where the ledger is.
- `KOTODEX_INGEST_DISABLE=1` — log to `lines.log` and post nothing.
- `VN_RUNDIR` (default `$XDG_RUNTIME_DIR/kotodex`) — where `lines.log` goes.
