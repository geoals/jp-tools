# Third-party components

What Kotodex ships, downloads or loads that someone else wrote, and under what
terms. Three groups, because they arrive three different ways.

## Checked into this repository

| what | used for | licence |
|---|---|---|
| preact, htm, @preact/signals — `web-shared/vendor/` | the dashboards' front end | MIT |
| KANJIDIC2 (via `davidluzgouveia/kanji-data`), reduced into `jp-core/src/text/kanji_data.rs` | school grade, stroke count and gloss per kanji | CC BY-SA 4.0 (Electronic Dictionary Research and Development Group) |
| BCCWJ character-frequency list (NINJAL), reduced into `jp-core/src/text/bccwj_data.rs` | the kanji frequency yardstick | see NINJAL's terms for the frequency lists at <https://pj.ninjal.ac.jp/corpus_center/bccwj/freq-list.html> |
| sudachi.rs resources — `jp-core/sudachi-resources/` (`sudachi.json`, `char.def`, `unk.def`, `rewrite.def`), WorksApplications | the tokenizer's own configuration and character tables | Apache-2.0 |

The two tables are reductions, not copies: the generator script and the source
URL are in each file's header. The sudachi.rs resources are copied unmodified from
that crate, and are shipped rather than found at runtime because its default
location is derived from the path of the crate on whatever machine compiled the
binary. The vendored libraries are the npm ESM builds
unmodified — see `web-shared/vendor/README.md` for versions and why they are
not on a CDN.

## Downloaded by `setup.sh`

Nothing here is redistributed: each is fetched from whoever publishes it, at the
version they publish today, which is also why the URLs are resolved rather than
pinned.

| what | used for | size | licence |
|---|---|---|---|
| SudachiDict full (`system_full.dic`), WorksApplications | tokenizing Japanese at all | 127 MB | Apache-2.0 |
| silero-vad ONNX model, snakers4/silero-vad | trimming a card's clip to the spoken line | 2.2 MB | MIT |
| Jitendex (`jitendex-yomitan.zip`), stephenmk | the popup's Japanese–English definitions | ~39 MB | CC BY-SA 4.0 |
| the Jiten frequency list, jiten.moe | which words are common in fiction — the underline, the rank pill, the sweep's order | ~8 MB | see <https://jiten.moe> |
| Kanjium pitch accents (`kanjium_pitch_accents.zip`), toasted-nutbread from mifunetoshiro/kanjium | the pitch notation in the popup | ~1 MB | CC BY-SA 4.0 |

Definitions and ranks are offered because without them the product is broken
rather than smaller: no definitions means an empty popup, and no ranks means
nothing is underlined or ordered. Pitch is a megabyte and no monolingual gloss
carries it.

## Loaded at run time, not distributed

- **Yomitan dictionary zips** the reader supplies in `dictionaries/`. Their terms
  are the dictionary's own. The commercial monolinguals — Sankoku, 明鏡, 小学館 —
  are not redistributable, so the installer neither ships nor fetches one and the
  vocabulary scale is simply not offered until the reader supplies a master.
- **The Anki note type.** Cards are written into whichever note type is
  configured; Lapis is the default field map. Its templates and CSS are not
  vendored here.
- **Rust crates**, resolved by Cargo; `cargo tree` and each crate's own
  metadata are the list. Almost all are MIT OR Apache-2.0.
- **Python packages** for the optional services (`whisper-service`,
  `manga-ocr-service`), from their `requirements.txt`. Notably faster-whisper
  (MIT) and kha-white's manga-ocr (Apache-2.0).

## Scope note

**Nothing sends reading data anywhere.** The one outbound call is to the
Anthropic API, and only when an API key is set and the explain button is pressed.

kotodex-server listens on `0.0.0.0:3200` on purpose, so a phone beside the screen can
open the same reading surface — `KOTODEX_SERVER_LISTEN_ADDR` narrows it to
`127.0.0.1:3200` where that is not wanted. There is no authentication, so treat
it as trusted-network only. Everything it talks *to* is loopback: AnkiConnect on
`127.0.0.1:8765`, the Local Audio Server on `:5050`, whisper-service on `:8100`.
