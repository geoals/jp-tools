# jp-tools

Cargo workspace for Japanese language learning tools.

- `jp-core/` — tokenization (Sudachi, hybrid Mode C/B with dictionary validation) + dictionary (Yomitan zip parsing, SQLite cache) library crate
- `yt-mine/` — YouTube sentence mining (Axum JSON API + Preact SPA, SQLite, Anki export). See `yt-mine/CLAUDE.md`
- `spec/` — feature specs and roadmap

```sh
cargo build           # all members
cargo test            # all members
cargo run -p yt-mine  # server
```
