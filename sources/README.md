# sources — where captured text enters the ledger

A source turns something being read into lines and posts them to
`POST /api/lines`. That is the whole contract. It may be written in anything,
and it does not have to run on the machine holding the database.

```
sources/textractor/   Textractor's WebSocket plugin, beside a VN on Linux
```

## The contract

```http
POST /api/lines
Content-Type: application/json

{
  "source": "textractor",
  "work": "余生",
  "lines": [
    { "text": "「ねえ、聞いてる？」", "ts": 1787749546.2, "ruby": [[0, 2, "かんじ"]] }
  ],
  "status": { "attached": true, "pending": 0 }
}
```

- `source` — a short token, letters, digits, `-` and `_`. It names the row in
  `lines.source` and is what a retract addresses. Default `vn`.
- `work` — for lines that don't name their own. Falls back to the dashboard's
  "now reading", which is what a VN source wants and a book source does not.
- `text` — required. Everything else on a line is optional: `ts` defaults to
  server time, and only a source replaying a backlog knows better.
- `ruby` — furigana as `[[start, len, reading]]` over `text`, in UTF-16 code
  units. Stored as given.
- `status` — the source's own health, for the capture badge. A source with
  nothing to send posts this alone.

The reply is `{"ids": [...], "paused": false}`. `POST /api/lines/retract` with
`{"source": "..."}` takes the last line back, for the case where the next
capture turns out to continue it.

## What a source does not decide

- **The character count.** `jp_core::text::chars` counts it from the text.
  Two implementations of that rule drift into two different answers for
  chars/h, which is the number every rate is built on.
- **Whether capture is paused.** `paused: true` comes back and the lines are
  dropped. Accepted rather than refused on purpose: a 4xx would make a source
  hold them and flush the whole paused span the moment capture resumed.
- **The schema.** No source opens the database. A column added to `lines` is
  jp-core's migration and nothing here needs to hear about it.

What a source *does* own is turning its own capture into a line: the hooker's
junk, a continuation split across two text boxes, its own dedup. None of that
generalises, and all of it is specific to where the text came from.

## Writing one

Post to the endpoint. There is nothing to register and nothing to build
against — a shell script with `curl` is a valid source. Hold what you cannot
send and retry, rather than dropping it: the server being restarted mid-session
is the failure that actually happens.
