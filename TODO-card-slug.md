# Jitendex is missing from every overlay-mined card

Handoff note for a machine that can reach AnkiConnect. **Delete this file in the
commit that fixes the bug.**

## The bug

`read-stats/src/routes/reader/mine.rs` filters the card's dictionaries through
`class_slug(title)` against `CARD_DICTIONARIES`:

```rust
const CARD_DICTIONARIES: [&str; 2] = ["三省堂国語辞典-第八版", "jitendex"];
```

`class_slug` lowercases and hyphenates whitespace, nothing else. The installed
Jitendex title is `Jitendex.org [2026-02-05]`, which slugs to
`jitendex.org-[2026-02-05]` — not `jitendex`. So the filter drops it and
**`VocabDefFull` gets Sankoku only.** Sankoku matches, which is why the field is
never empty and the loss is silent.

Confirmed live: `GET /api/reader/define?term=言葉&reading=ことば` returns both
dictionaries, and the card keeps one. The `#[test]`s pass because they assert
`class_slug("Jitendex")`, the bare title the dictionary no longer has.

The date in the title is the trap: any slug derived from it changes when
Jitendex is updated, so deriving the class name is what broke and re-deriving it
correctly would break again on the next release.

## The fix

The class name is whichever `.dict-*` rule the note type has CSS for. That is a
fact about the Anki note type, not a function of the title — so hardcode the
pair, matching on title prefix:

```rust
/// The two dictionaries the card carries, each with the class name the note
/// type's CSS lists.
///
/// The class name is not derived from the title. Jitendex's title carries its
/// release date (`Jitendex.org [2026-02-05]`), so a slug built from it changes
/// on every update and silently stops matching — which is exactly how Jitendex
/// came to be missing from every card while the field still looked full.
const CARD_DICTIONARIES: [(&str, &str); 2] = [
    ("三省堂国語辞典", "<what the CSS says>"),
    ("Jitendex", "<what the CSS says>"),
];
```

Then `dict_block` takes the class name as an argument instead of calling
`class_slug`, and `class_slug` is deleted along with its test.

## What has to be checked first

Read the note type's CSS and use the class names it actually styles:

```sh
curl -s localhost:8765 -X POST \
  -d '{"action":"modelStyling","version":6,"params":{"modelName":"Japanese sentences"}}' \
  | grep -o '\.dict-[^ ,{:>]*' | sort -u
```

`read-stats/CLAUDE.md:315` claims `.dict-jitendex-body` and mentions
`.dict-sanseido-body` as a shorter alias sitting beside
`.dict-三省堂国語辞典-第八版-body`. **Verify rather than trust it** — that doc
line also documents `class_slug` as correct, and it is not. If the CSS has both
an alias and a full-title rule, prefer the one that does not embed a version.

## Verify

The card is what matters, so check the real thing end to end:

1. `cargo test -p read-stats`
2. Mine a word from the overlay that both dictionaries hold (言葉 works).
3. In Anki, confirm `VocabDefFull` now has **two** `dict-*-title` blocks, and
   that Jitendex's star and its ① ② numbering are still hidden — that styling is
   the whole reason the wrapper exists, so an unstyled second block means the
   class name is wrong.

`scripts/dev-instance.sh` is the safe way to do this if a VN is being read.

## Also update

- `read-stats/CLAUDE.md:315-321` — the `class_slug` sentence and the
  `CARD_DICTIONARIES` bullet. Say the class name is fixed per dictionary and
  why, so the next reader does not reintroduce a derivation.

## Not part of this

The other three suspected duplications were checked and are **not** duplicates.
Don't merge them:

- `bold_surface` vs `jp_mine_core::lookup::bold_target_in_sentence` — different
  inputs (a surface string from the client vs `&[Token]`), and jp-mine-core's
  does not HTML-escape.
- `dict_block` vs `Dictionary::wrap_definitions` — different HTML.
  `wrap_definitions` emits `dict-section`/`dict-title`; the card needs Yomitan's
  `yomitan-glossary` `<ol>` nesting, held to a byte-for-byte test against a card
  Yomitan wrote.
- `html_escape` — **already done**, in the commit that added this file.
  `jp_core::dictionary::html::html_escape` is now public and `mine.rs` uses it;
  the local copy that missed quotes is gone.
