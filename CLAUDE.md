# jp-tools — Monorepo

Cargo workspace containing Japanese language learning tools.

## Projects

| Directory | Description |
|---|---|
| `yt-mine/` | YouTube sentence mining — Axum server + htmx frontend, SQLite, Anki export |

## Workspace layout

```
Cargo.toml          — workspace manifest
Cargo.lock          — shared lockfile
.env / .env.example — runtime config (stays at root)
spec/               — feature specs and roadmap
yt-mine/            — sentence mining tool (see yt-mine/CLAUDE.md)
```

## Build & run

```sh
cargo build              # build all workspace members
cargo test               # test all workspace members
cargo run -p yt-mine     # run the yt-mine server
```
