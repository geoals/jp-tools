# Third-party components

What Kotodex ships, downloads or loads that someone else wrote, and under what
terms. Three groups, because they arrive three different ways.

## Checked into this repository

| what | used for | licence |
|---|---|---|
| preact, htm, @preact/signals — `web-shared/vendor/` | the dashboards' front end | MIT |
| KANJIDIC2 (via `davidluzgouveia/kanji-data`), reduced into `jp-core/src/text/kanji_data.rs` | school grade, stroke count and gloss per kanji | CC BY-SA 4.0 (Electronic Dictionary Research and Development Group) |
| BCCWJ character-frequency list (NINJAL), reduced into `jp-core/src/text/bccwj_data.rs` | the kanji frequency yardstick | see NINJAL's terms for the frequency lists at <https://pj.ninjal.ac.jp/corpus_center/bccwj/freq-list.html> |

The two tables are reductions, not copies: the generator script and the source
URL are in each file's header. The vendored libraries are the npm ESM builds
unmodified — see `web-shared/vendor/README.md` for versions and why they are
not on a CDN.

## Downloaded by `setup.sh`

| what | used for | size | licence |
|---|---|---|---|
| SudachiDict full (`system_full.dic`), WorksApplications | tokenizing Japanese at all | 127 MB | Apache-2.0 |
| silero-vad ONNX model, snakers4/silero-vad | trimming a card's clip to the spoken line | 2.2 MB | MIT |

## Loaded at run time, not distributed

- **Yomitan dictionary zips** the reader supplies in `dictionaries/`. Their terms
  are the dictionary's own. Kotodex ships none and downloads none — Jitendex is
  CC BY-SA 4.0 and freely redistributable, and the commercial monolinguals are
  not, which is why the installer only says where to get them.
- **The Anki note type.** Cards are written into whichever note type is
  configured; Lapis is the default field map. Its templates and CSS are not
  vendored here.
- **Rust crates**, resolved by Cargo; `cargo tree` and each crate's own
  metadata are the list. Almost all are MIT OR Apache-2.0.
- **Python packages** for the optional services (`whisper-service`,
  `manga-ocr-service`), from their `requirements.txt`. Notably faster-whisper
  (MIT) and kha-white's manga-ocr (Apache-2.0).

## Scope note

Everything binds to loopback. The AnkiConnect proxy in read-stats forwards to
`127.0.0.1:8765` and is reachable only from this machine; nothing in the stack
listens on a public interface or sends reading data anywhere. The one outbound
call is to the Anthropic API, and only when an API key is set and the explain
button is pressed.
