# jp-tools

Cargo workspace for Japanese language learning tools.

- `yt-mine/` — YouTube sentence mining (Axum + htmx, SQLite, Anki export). See `yt-mine/CLAUDE.md`
- `spec/` — feature specs and roadmap

```sh
cargo build           # all members
cargo test            # all members
cargo run -p yt-mine  # server
```
