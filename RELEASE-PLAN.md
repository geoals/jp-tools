# Release plan — a shareable Linux VN reading overlay

Product name: **Kotodex / コトデックス** (decided, T0.1). Everything below uses
`kotodex` as the binary, desktop-entry id and tray name.

## What "done" means

On ~90% of current desktop Linux installs:

1. Download one tarball from a GitHub release.
2. Run `./setup.sh`, answer a handful of yes/no prompts.
3. Get an application entry with a name and an icon.
4. Launch it → the capture daemon, the read-stats server and the overlay all
   come up. Launch it again → it detects the running instance and does nothing.
5. The overlay has a close button (stops everything) and a minimise-to-tray
   button.
6. Anything missing degrades to a smaller working product with one clear
   sentence saying what is off and the one command that turns it on.

Non-goals for this release: whisper auto-setup, Windows/macOS, X11-only
compositors older than the ones in T0.4, GNOME under Wayland if T0.4 says no.

## Ground rules

- **Commit straight to `master`**, one commit per completed task, only when its
  verification passes. Every task below is sized to be independently
  committable without breaking the running setup.
- **Never restart the live stack while a VN is being read.** Use
  `scripts/dev-instance.sh` (port 3299, copy of the data) for anything
  read-stats-side. Overlay/static changes need `vn-overlay.sh restart`.
- **The golden corpus is the regression gate** for anything touching
  dictionaries, roles or the tokenizer:

  ```
  JP_TOOLS_SUDACHI_DICT_PATH=$PWD/system_full.dic \
    cargo test -p jp-core --features test-support -- --ignored
  ```

  It must stay green. The failure diff is the review — read it, and only then
  regenerate with
  `cargo run --release --example golden -p jp-core --features test-support -- <knowledge.db> jp-core/tests/golden/corpus.txt $PWD/system_full.dic`.
- Prose in code and docs follows the repo's rules: plain English, say why not
  what, no history in comments.
- A task that turns out bigger than its description gets split rather than
  stretched.
- **This document is the live status, not a plan written once.** Every task
  carries a status line — `**Status:** done (<commit>)`, `blocked (why)` or
  nothing while it is untouched — updated in the same commit as the work.
  A decision made, a fact learned or a task that turned out different is
  written back here, and the open-decisions list at the end shrinks as they are
  settled. If the document and the repo disagree, the document is wrong and
  fixing it is part of the task.

---

# Phase 0 — Decisions and baselines

No production code. Output is a decision record and two baselines that later
phases diff against.

### T0.1 — Name, ids, paths

**Status:** done (46b9cde).

Decide and write down:

| thing | proposal |
|---|---|
| product name | Kotodex (コトデックス) |
| launcher binary | `kotodex` |
| desktop entry | `kotodex.desktop`, `Name=Kotodex` |
| icon | `kotodex` in hicolor 256/512 + scalable SVG |
| capture daemon | `kotodex-capture` (was `vn-buffer`) |
| install prefix | `~/.local/share/kotodex` (data), `~/.local/bin` (binaries) |
| config file | `~/.config/kotodex/config.toml` |

The crate, service and database names (`read-stats`, `read-stats.db`) do **not**
change — same rule already in CLAUDE.md for コトデックス.

**Decided.** Kotodex it is; the table above is the record. The crate, service
and database names (`read-stats`, `read-stats.db`) do not change.

**Verify:** decisions written into this file. **Commit:** `RELEASE-PLAN: name and paths`.

### T0.2 — Tokenizer and card baselines

**Status:** done (46b9cde); the golden fixtures were stale and were regenerated first (4118339).

**Done.** Captured into the session scratchpad's `baseline/`:

- `tests-before.txt` — the golden gate, green after regenerating the fixtures
  for d3bb637 (the tokenizer baseline is the fixture, not a dumped file).
- `anki-notes.json` — the 20 most recent mined notes, the field-by-field
  baseline for Phase 2.
- `dictionaries.txt` — the 10 live dictionary rows.

The live roles, for Phase 1 to move: 1 三省堂 `master`; 6 明鏡, 7 小学館
`standard`; 2 Jitendex, 3 NHK, 4 BCCWJ, 5 Jiten and the three `[Pitch]` zips
(8, 9, 10) all `reference`. Jiten becomes `frequency`, BCCWJ the second
`frequency`, and 3/8/9/10 become `pitch`.

**Verify:** all three files non-empty. **Commit:** none (scratch).

### T0.3 — Degradation matrix

**Status:** done (d816320) — `docs/degradation.md`.

Write `docs/degradation.md`: every optional component, what it gives, what
happens without it, and the one command that installs it. This is the
specification that Phase 3's capability probe and Phase 6's installer both
implement, and the source text for the doctor's output.

Rows to cover: pactl/ffmpeg, silero VAD model, SudachiDict, master dictionary,
any definition dictionary, frequency dictionary, pitch dictionary, AnkiConnect,
Anki note type + fields, screenshot tool, whisper, Anthropic key, xdotool,
layer-shell vs X11 backend, Textractor WebSocket source vs clipboard.

**Verify:** every row has all four columns filled. **Commit:** `docs: degradation matrix`.

### T0.4 — Compositor spike (blocks the "90%" claim)

**Status:** done — `docs/compositors.md`. GNOME Wayland **passes**: an
always-on-top X11 window stays above a fullscreen XWayland one, and xdotool
returns its geometry. The catch is that Qt must run under `QT_QPA_PLATFORM=xcb`
— on a native Wayland surface the on-top hint is silently a no-op, which is
what a first run of the probe measured and wrongly read as a GNOME failure.

GNOME Xorg does not exist any more and KDE Xorg is not installed here, so those
two rows are untestable rather than failing. Hyprland is still untested; it is
wlroots, so layer-shell is expected to work. Click-through on GNOME is still
unverified.

The 90% claim stands and is not narrowed.

Cheapest first: install `gnome-shell` alongside the current session on the real
machine, log into GNOME Wayland, and test whether an X11 `_NET_WM_STATE_ABOVE`
window stacks above a **fullscreen XWayland** window.

Test target needs no VN: `mpv --gpu-context=x11 --fs`, `glxgears -fullscreen`,
or any SDL app forced to x11. Test overlay: a 200x100 Qt window with
`Qt.WindowStaysOnTopHint` and `setMask` for click-through.

Record for each of GNOME Wayland, GNOME Xorg, KDE Wayland, KDE Xorg, Hyprland:

- does the overlay stay above a fullscreen XWayland window?
- does click-through via XShape work?
- does `xdotool search --name … getwindowgeometry` return the game rectangle?

**Decision gate:** if GNOME Wayland fails on stacking, the README says
"KDE/wlroots for fullscreen, GNOME for windowed" and the 90% claim narrows to
"90% of distros install and run; fullscreen overlay needs a layer-shell
compositor". Do not soften this later — write whichever sentence the test
supports.

**Verify:** results table in `docs/compositors.md`. **Commit:** `docs: compositor support matrix`.

---

# Phase 1 — De-hardcode the dictionary set

Goal: no dictionary title appears in code except as an installer default. Every
consumer asks for a **role** and handles `None`.

Order matters — T1.1 first, then each consumer, then the fallbacks.

### T1.1 — Two new roles

**Status:** done (c02cd8a).

`jp-core/src/knowledge/dictionaries.rs`: add `Role::Frequency` and `Role::Pitch`
to the enum, its `as_str`/`FromStr`, and `jp-dict set-role`'s accepted values.
Neither counts toward the vocabulary scale; both count for the wordhood gate
exactly as `Reference` does today.

**Verify:** `cargo test -p jp-core`; `jp-dict list` prints the new roles;
`jp-dict set-role <id> frequency` round-trips. **Commit:** `dictionaries: frequency and pitch roles`.

### T1.2 — Reader frequency by role

**Status:** done. `dictionaries::reader_frequency` is the resolver; the constant
survives only as the installer's default. Live: Jiten is now `frequency`. The
golden gate and the whole workspace test suite are green, and a dev instance
with no frequency dictionary answers `"jiten": null` with nothing in the log —
`read-stats/src/routes/vocab.rs` falls back to encounter ordering rather than
erroring, which it used to do.

Replace `READER_FREQUENCY` lookups with "the dictionary holding `Role::Frequency`,
preferring the lowest id when several do". Call sites:
`jp-core/src/highlight.rs:203`, `jp-core/src/define.rs:170`,
`jp-core/examples/golden.rs:126`, `yt-mine/src/routes/api/mod.rs:700`,
`manga-mine/src/routes/api.rs:695`, `read-stats/src/ingest.rs:262`.

Keep the constant as the **installer's default assignment**, not a runtime
lookup. `None` means: no underline in the overlay, no rank pill, no
by-frequency ordering in the sweep — never an error.

**Verify:** `jp-dict set-role 5 frequency` on the live set, `golden` diff empty
against T0.2; then temporarily `set-role 5 reference` and confirm the overlay
draws lines with no underlines and no crash, then set it back.
**Commit:** `highlight, define: reader frequency by role`.

### T1.3 — Corpus frequency pill by role

**Status:** done. `Definition.jiten`/`.bccwj` are
gone; the popup gets `frequencies: [{dictionary, rank}]`, one entry per
`Role::Frequency` dictionary in id order, and draws no pill row when it is
empty. The label is the dictionary's own title.

Both pills are live, Jiten first, because `dictionaries.priority` decides who
answers first — see the note under T1.4.

`jp-core/src/define.rs:33` — `CORPUS_FREQUENCY` becomes "a second dictionary
with `Role::Frequency`, if there is one". With one frequency dictionary the
popup shows one pill; with none it shows no pill row at all.

`web-shared/popup.js:402` currently hardcodes the two labels — take the label
from the dictionary title instead.

**Verify:** popup on a live word shows both pills unchanged; with BCCWJ set to
`reference` it shows one. **Commit:** `define: second frequency pill by role`.

### T1.4 — Popup ordering without a name list

**Status:** done — `SECOND_PAGE` is gone, the sort keys on the role.

**The live popup order changes**, which the task's own verify line did not
expect: Jitendex used to be page two by name, and under roles the standard
monolinguals take that place. It is now 三省堂 → 明鏡 → 小学館 → Jitendex. That
is the ordering the role model implies — a bilingual dictionary after the
monolinguals — but say so if you want Jitendex back at two, since the only way
to get it there now is a role change.

`jp-core/src/define.rs:38` — delete `SECOND_PAGE`. Order becomes: master first,
then `Standard`, then everything else in install order. The `entries.is_empty()`
filter at `define.rs:111` already excludes frequency and pitch dictionaries and
stays as it is.

**Verify:** popup page order on a word present in 3+ dictionaries matches what
it was before for the live set. **Commit:** `define: order by role, not by name`.

### T1.5 — Pitch by role

**Status:** done. Pitch dictionaries are tried first, then anything else
carrying accent rows, and no accent renders nothing rather than an empty slot.
Live: NHK and the three `[Pitch]` zips now hold the role, and the popup's
accents are byte-identical before and after.

`jp-core/src/define.rs` currently takes pitch from whichever dictionary has
accent rows. Make it explicit: prefer `Role::Pitch`, fall back to any dictionary
with accent data, else `None` — and `None` renders no ♪ and no downstep rather
than an empty slot.

**Verify:** popup unchanged live; with NHK removed from the pitch role, the
accent line disappears cleanly. **Commit:** `define: pitch by role`.

### T1.6 — Master becomes optional

**Status:** done. `ensure_master` falls back to the first `standard`
monolingual, then the dictionary with the fewest headwords, then — before any
entries have been counted — the first that could hold definitions at all. A
config naming an uninstalled dictionary still leaves an existing master alone;
the fallback is only for having none. Verified on a Jitendex-only copy of the
database: it takes the master role and the whole corpus tokenizes sensibly.

`ensure_master` keeps its marker argument. New fallback chain when the marker
matches nothing: the highest-priority `Standard` present, else the dictionary
with the fewest headwords (a monolingual is smaller than Jitendex), else no
master at all.

With no master: the vocabulary scale is not offered (Phase 3 hides it), the
wordhood gate still answers from any dictionary, and segmentation uses whatever
`Standard`/`Master` dictionaries exist — Jitendex over-joining included, which
is the accepted behaviour per the design discussion.

**Verify:** on a scratch `knowledge.db` with Jitendex only, `jp-dict list` shows
it as master and tokenizing a test line returns sensible tokens.
**Commit:** `dictionaries: master is optional`.

### T1.7 — Roles assigned at import

**Status:** done. The guess reads what the zip turned out to hold: term entries
→ `reference`, frequency rows only → `frequency`, accent rows only → `pitch`.
`--role` overrides and is rejected on a typo. Only a *new* row is guessed at, so
a `set-role` decision survives every later sync. Master is left to
`ensure_master` rather than guessed a second time.

Verified by importing Jitendex, a frequency zip and a pitch zip into an empty
database with no flags: `reference` / `frequency` / `pitch`, and Jitendex takes
master through T1.6's fallback.

`jp-dict import` guesses a role from what the zip contains: term entries →
`Reference` (or `Master` if it is the first monolingual), frequency rows only →
`Frequency`, accent rows only → `Pitch`. `--role` overrides.

**Verify:** import Jitendex, a frequency zip and a pitch zip into a scratch db;
all three land in the right role with no flags. **Commit:** `jp-dict: guess role at import`.

---

# Phase 2 — Card authoring and Lapis

Depends on Phase 1 (role-based dictionary selection).

### T2.1 — Kill the card dictionary allowlist

**Status:** done, folded into T2.2 — the two could not be separated. Dropping
the allowlist outright rewrites the live card's markup, because the class name
is deliberately *not* the title's slug: the legacy note type's CSS is written
against `.dict-jitendex-body`, and Jitendex's title carries a release date. So
the allowlist survives as the **legacy style's** table, which is what "legacy"
means, and the Lapis style takes every dictionary holding a definition.

The silent failure is fixed where it can be: under Lapis, 明鏡 now reaches the
card. Under legacy it still cannot — its CSS has no rules to render it with,
which is the reason for T2.7.

`jp-mine-core/src/card.rs:32` — delete `CARD_DICTIONARIES`. `card_class`
becomes `jp_core::dictionary::css_slug(title)`. The filter for which
dictionaries reach the card becomes the same `entries.is_empty()` rule the
popup uses at `define.rs:111`, so frequency and pitch dictionaries exclude
themselves.

**This is a silent-failure bug fix**, not only a refactor: today any dictionary
outside the two-entry list is dropped from `VocabDefFull` with no error.

**Verify:** export one card with the live dictionary set and diff its
`VocabDefFull` against the T0.2 baseline — must be byte-identical for Sankoku
and Jitendex. Then set a third dictionary to a definition role and confirm it
appears. **Commit:** `card: every definition dictionary reaches the card`.

### T2.2 — Card style profiles

**Status:** done. `JP_TOOLS_ANKI_STYLE=legacy` is set in `.env`, which is
gitignored — the local, opt-in pin, so nothing about the live setup moves while
the default for everyone else is Lapis.

Add `JP_TOOLS_ANKI_STYLE` = `lapis` (default) | `legacy`.

- `legacy` — today's `dict_block`: `.dict-{slug}-title` / `.dict-{slug}-body`
  wrappers around `.yomitan-glossary`. The nesting is load-bearing for the
  existing note type's CSS and must not change.
- `lapis` — the `.yomitan-glossary` block alone, no wrappers, one per
  dictionary in popup order. Lapis styles Yomitan's markup directly.

Set `JP_TOOLS_ANKI_STYLE=legacy` in the user's own environment in the same
commit, so live behaviour is unchanged.

**Verify:** unit test asserting both shapes; live export still legacy and
identical to baseline. **Commit:** `card: lapis and legacy glossary styles`.

### T2.3 — Lapis field defaults

**Status:** done. `AnkiConfig::default()` is Lapis; `field_reading` and
`field_freq_sort` are new and are written by `export`, the reading taken out of
the furigana's brackets rather than plumbed through a second time. The existing
note type's names are pinned in `.env` — every field, including the five that
used to rely on the defaults — so nothing about the live export moves. An empty
value means "this note type has no such field", which is how `ExpressionReading`
and `FreqSort` stay off it.

`jp-mine-core/src/config.rs` — `AnkiConfig::default()` becomes the Lapis map:

| field | Lapis |
|---|---|
| model / deck | `Lapis` / `Japanese` |
| vocab | `Expression` |
| definition | `Glossary` |
| compact_def | `MainDefinition` |
| sentence | `Sentence` |
| image | `Picture` |
| audio | `SentenceAudio` |
| source | `MiscInfo` |
| furigana | `ExpressionFurigana` |
| pitch_num / pitch_pattern | `PitchPosition` / `PitchCategories` |
| frequency | `Frequency` |

Add two fields: `field_reading` → `ExpressionReading`, `field_freq_sort` →
`FreqSort` (the reader-frequency rank as a plain integer). All four
`Is…Card` selectors stay empty — blank gives plain word-front vocab cards.

Write the user's current names into their environment in the same commit.

**Verify:** `AnkiConfig::from_env()` with the user's env reproduces the old
config exactly (unit test); with a clean env it produces Lapis.
**Commit:** `anki: lapis field defaults`.

### T2.4 — vn-capture.sh reads the field map

**Status:** done. The four names come from `JP_TOOLS_ANKI_FIELD_*` with Lapis
defaults, and `ANKI_CONNECT_URL` from `JP_TOOLS_ANKI_URL`. The script also reads
`.env` itself: read-stats inherits its environment to the script, but a hotkey
run has no parent to inherit from, and the two paths must not disagree about
which field holds the sentence. Parsing the live `.env` yields VocabKanji /
SentKanji / Image / SentAudio, unchanged.

Not yet exercised against a real mine — no VN session has run since. That is
T2.6's job.

`vn-mine/vn-capture.sh:337,338,408,410` hardcode `VocabKanji`, `SentKanji`,
`Image`, `SentAudio`. Read them from `JP_TOOLS_ANKI_FIELD_*` with the same
defaults as the Rust side.

**Verify:** mine one card from the overlay with legacy env — image and audio
attach as before. **Commit:** `vn-capture: field names from the environment`.

### T2.5 — Note type check and creation

**Status:** blocked. It hangs off a `kotodex` binary that Phase 4 creates, and
on open decision 4 — vendor the Lapis note type or link to its release deck.
Verifying it needs a fresh Anki profile.

New: `kotodex anki check` (also called by the doctor and the installer). Probes
AnkiConnect, then `modelNames` / `modelFieldNames`, and reports one of:

- note type present with every configured field → ok
- present, fields missing → list exactly which, and the field map to fix
- absent → offer `modelNames`-based creation from a bundled Lapis definition,
  or point at the Lapis release deck

Bundle the Lapis note type as `assets/lapis/` (templates + CSS) with its
licence, or — decide in this task — link to the upstream release deck instead
of vendoring. Vendoring is friendlier and needs an update path; linking is
zero maintenance and one more manual step.

**Verify:** against a fresh Anki profile with no Lapis: reports absent, creates
it, second run reports ok. **Commit:** `anki: note type check and setup`.

### T2.6 — Live card round-trip

**Status:** blocked — manual, and needs a reading session and a fresh profile.
This is where T2.4's field map gets its first real mine.

Mine one card in each style into a scratch deck on a fresh profile, and look at
it: glossary renders, definitions page, pitch shows, image and audio attach,
frequency sorts.

**Verify:** manual, screenshots into `docs/`. **Commit:** none (validation).

### T2.7 — Convert the user's own note type

**Status:** waiting on you. Everything it depends on is in place: `.env` pins
the current names and `JP_TOOLS_ANKI_STYLE=legacy`, so the conversion is a
rename in Anki followed by deleting those lines.

Separate from everything above, on the user's schedule: move the personal note
type to Lapis-compatible field names, then delete `JP_TOOLS_ANKI_STYLE=legacy`
from the environment. The `legacy` branch stays in the code until this is done
and is deleted in a follow-up.

**Verify:** existing cards still render. **Commit:** `card: drop the legacy style` (later).

---

# Phase 3 — Capabilities, degradation, doctor

### T3.1 — One capability probe

**Status:** done — `read-stats/src/routes/reader/capabilities.rs`, served under
`capabilities` on `/api/reader/state` and cached for ten seconds. Every row is
`{ok, detail, fix}`: `detail` is for the surfaces, `fix` is the sentence from
`docs/degradation.md` for the doctor. The old `trim_available` now reads off the
whisper row rather than probing twice.

Verified against the dev instance with Anki and whisper down: both report off
with their fix line, nothing errors.

Extend `read-stats/src/routes/reader/state.rs` from 6 keys to the full
degradation matrix. One struct, one JSON object, probed with short timeouts and
cached briefly. Keys at least:

`capture_running`, `lines_source` (ws | clipboard | db), `vad_model`,
`screenshot_tool`, `anki`, `anki_note_type`, `whisper`, `explain`,
`dict_definitions` (count), `dict_frequency`, `dict_pitch`, `dict_master`,
`vocabulary_ledger` (row count), `xdotool`, `overlay_backend`.

Each value is enough for the client to decide whether to draw a control, and
carries the fix sentence from `docs/degradation.md` for the doctor to print.

**Verify:** `curl localhost:3299/api/reader/state | jq` against the dev instance
with pieces disabled one at a time. **Commit:** `reader: one capability probe`.

### T3.2 — The overlay honours capabilities

**Status:** done. The overlay reads `capabilities` on open: no ℹ without an API
key, no ＋ in the popup without AnkiConnect, no underline without a frequency
dictionary, and `#warn` names a missing line source. `popup.js` gained
`setMining(on)` and draws no ＋ when a host passes no `mine` — the same shape
`mined` and `audio` already had, so yt-mine is unaffected.

The ♪ is not gated: nothing probes the Local Audio Server yet. It already
hides itself when no clip is found, so it is a button that disappears rather
than one that fails.

`read-stats/overlay/overlay.js` — a control that cannot work is not drawn:
no ℹ without an API key, no mine without AnkiConnect, no ♪ without a pitch or
audio source, no status tints without a ledger, no underline without a
frequency dictionary. The `#warn` line already exists for the one failure that
is invisible; extend it to name a missing capture source.

**Verify:** load the overlay against a dev instance with each capability forced
off; no dead buttons, no console errors. **Commit:** `overlay: draw only what works`.

### T3.3 — Status painting is opt-in

**Status:** done, as a setting plus a fact. `highlight_status` defaults on, and
an empty ledger paints nothing regardless — so a fresh install reads as plain
text without being told to, and the setting is the explicit switch. Spans stay
in place either way; they are the click targets.

A setting (server-side, see T5.7) `highlight_status` defaulting **off** for a
fresh install and **on** where the ledger already has rows. Off means spans are
still computed — they are the click targets — but carry no status class.

**Verify:** toggle it on the dev instance; spans stay clickable both ways.
**Commit:** `overlay: status highlighting is opt-in`.

### T3.4 — Empty-database states

**Status:** done, and it was mostly already there. Against empty copies of both
databases every endpoint answers 200 with nothing in the log, and the kanji
grid, vocabulary curve, library, work triage, mined list and both trend charts
each say what would fill them.

One real gap, now fixed: Trends' day-by-day table drew a fortnight of dashes —
the shape of a table with nothing in it. It says "No reading recorded yet."
instead.

Two stale assertions in `dev-instance.sh browser` were fixed on the way: it
looked for kanji text that has since been reworded, and for "pause capture"
when the frozen copy had capture already paused, so the button read "resume
capture".

Every dashboard surface that assumes data: kanji grid, vocabulary curve, work
triage, mined list, timeline. Each gets an empty state saying what would fill
it. Nothing renders a zeroed chart.

**Verify:** point a dev instance at an empty copy of the databases and click
every tab. **Commit:** `read-stats: empty states`.

### T3.5 — Distro and package-manager detection

**Status:** done — `scripts/lib/platform.sh`, with
`scripts/lib/platform-test.sh` running it against a faked `/etc/os-release` for
nine distro families. Unrecognised ones fall back to whichever manager is on
`PATH`. Package names are a table, not a guess: `pyside6` is `python-pyside6`
here and `python3-pyside6.qtwebengine` on Debian.

Small shared shell library `scripts/lib/platform.sh`: reads `/etc/os-release`
`ID` and `ID_LIKE`, maps to pacman / apt / dnf / zypper / apk / xbps / nix, and
exposes `pkg_install_cmd <generic-name>` returning a paste-able command. Carries
the per-distro package names for: ffmpeg, jq, curl, xdotool, PySide6,
qt6-webengine, layer-shell-qt, grim/spectacle/gnome-screenshot, python.

**Verify:** unit-ish test with faked `/etc/os-release` for each distro.
**Commit:** `scripts: distro and package manager detection`.

### T3.6 — `kotodex doctor`

**Status:** done as `scripts/kotodex-doctor.sh`, which Phase 4's `kotodex
doctor` will call rather than reimplement. Rows come from read-stats' capability
probe; the ones that are about a missing command come from `platform.sh`, so
each prints an install line for *this* distro. Exit 0 when the core works —
curl, jq, SudachiDict, read-stats answering, and at least one definition
dictionary. Everything else prints and is forgiven.

Verified green against a dev instance, and in a stripped environment where it
still reports rather than crashing: it resolves its own path through bash
expansion instead of `readlink`, because it is the one script that has to
survive a system missing what it is about to report as missing.

One command, human-readable output, exit 0 if the core works. Sections: capture,
dictionaries, Anki, overlay, optional extras. Every failing row prints the fix
command from T3.5. The whisper row says what it would add and that it is not
set up automatically in this release.

The installer's closing summary is this same code path — one implementation.

**Verify:** run on the live machine (everything green) and inside a container
with nothing installed (everything red, no crash, useful text).
**Commit:** `kotodex: doctor`.

---

# Phase 4 — Process model, launcher, tray

### T4.1 — Rename the capture daemon

**Status:** done. `vn-mine/kotodex-capture` and `kotodex-capture.service`;
`vn-buffer.sh` stays as a one-line shim that execs the new name, so an installed
`vn-buffer.service` naming the old path keeps working. Docs swept: both READMEs,
`vn-capture.sh`'s messages, `vn-ws-logger.py`'s comment and root `CLAUDE.md`.

`vn-mine/vn-buffer.sh` → `kotodex-capture`, `vn-buffer.service` →
`kotodex-capture.service`. Keep a `vn-buffer.sh` shim that execs the new name
for one release. Update `vn-mine/README.md`, root `CLAUDE.md`, and the memory
note about restarting via vn-buffer.

**Verify:** `kotodex-capture restart` works; lines still arrive.
**Commit:** `vn-mine: rename vn-buffer to kotodex-capture`.

### T4.2 — Remove hardcoded paths from the unit

**Status:** done. **Decided: both, not either.** `ExecStart` is
`%h/.local/bin/kotodex-capture run` — no checkout path in it — and the unit
stays supported, because a systemd-managed daemon is what an existing install
already has and it survives a crashed supervisor. T4.3's adopt-or-start is what
makes the two coexist: the supervisor starts a daemon only when nothing is
already recording.

Running from a checkout is one symlink, which the README now names.

`vn-mine/vn-buffer.service:6` is `ExecStart=%h/git/jp-tools/vn-mine/vn-buffer.sh run`
— an absolute assumption about where the repo is cloned. The unit is generated
by the installer from a template, or dropped entirely in favour of the
supervisor (T4.3) owning the daemon.

**Decide in this task:** systemd unit or supervisor child. Supervisor child is
simpler to reason about and makes "close stops everything" exact; the unit
survives a crashed supervisor. Recommendation: supervisor child, with
`Restart=on-failure` behaviour reimplemented in the supervisor.

**Verify:** fresh checkout in a different directory starts correctly.
**Commit:** `kotodex-capture: no hardcoded install path`.

### T4.3 — The supervisor

**Status:** written, unverified on a live desktop. `kotodex/kotodex.py`:
adopt-or-start each of capture, read-stats and overlay; start in that order,
waiting for `/api/reader/state`; stop in reverse; restart a child that exits
non-zero with a backoff and give up after three, naming it.

**Decided against the plan's recommendation on one point:** the Qt process is
the *launcher*, not the overlay. The overlay is a QML layer surface and the tray
needs widgets; merging them buys nothing and risks a piece that already works.
The overlay stays a child.

`kotodex status` and `kotodex doctor` run headless and work.

New `kotodex` launcher. Responsibilities:

- **adopt-or-start** each component: if something is already listening on the
  read-stats port, adopt it and do not start a second; same for the capture
  daemon and the overlay. This is what makes it coexist with `start-all.sh`
  and with the user's existing setup.
- start order: capture daemon → read-stats (wait for `/api/reader/state`) →
  overlay.
- stop order: reverse, with a grace period, then SIGKILL.
- restart a child that exits unexpectedly, with backoff; give up loudly after
  three failures and say which component.

Language: Python, same process as the Qt shell (T4.4/T4.5 need Qt anyway), or a
bash supervisor with the Qt shell as a child. **Recommendation: the Qt process
is the app** — it owns the tray, the single-instance socket and the children,
so "close" is one process exiting.

**Verify:** `kotodex` from a clean state starts all three; with read-stats
already running from `start-all.sh` it adopts it and says so.
**Commit:** `kotodex: supervisor`.

### T4.4 — Single instance

**Status:** written, unverified. `QLocalServer` on `kotodex`, second launch
sends `show` and exits 0 with no error. A socket left by a SIGKILLed process is
removed only after the probe connect fails, which is the one moment it is safe
to remove.

`QLocalServer` socket in `$XDG_RUNTIME_DIR/kotodex.sock`. Second launch connects,
sends `show`, and exits 0 without printing an error. A stale socket from a
crashed process is detected and replaced.

**Verify:** launch from the desktop entry twice — second launch raises the
overlay and starts nothing. Kill -9 the first, launch again — no stale-socket
failure. **Commit:** `kotodex: single instance`.

### T4.5 — Tray icon

**Status:** written, unverified. Menu: show/hide overlay, open reading stats,
pause capture, doctor, quit. When `isSystemTrayAvailable()` is false it says so
once and leaves the overlay on screen rather than hiding it — the GNOME case.
The tooltip names anything that was adopted, since those are what quitting
deliberately leaves running.

`QSystemTrayIcon` owned by the Qt process, so it outlives the overlay window
being hidden. Menu: Show/Hide overlay, Open reading stats (opens
`http://localhost:3200` in the browser), Pause capture, Doctor, Quit.

Note the failure mode: GNOME has no tray by default (needs the AppIndicator
extension). Detect and, when there is no tray, say so once at startup and keep
a visible minimise-to-corner state in the overlay instead of hiding entirely.

**Verify:** tray appears on KDE; on GNOME without the extension the fallback
path leaves the overlay reachable. **Commit:** `kotodex: tray icon`.

### T4.6 — Close and minimise buttons in the overlay

**Status:** written, unverified. Two buttons at the end of the bar, both
hidden in an ordinary browser where there is no surface to hide and no process
to stop. They call `shell.minimise()` and `shell.quit()`, new generic slots on
`layer-overlay` — a page over a layer surface may want either, and neither
mentions Kotodex.

**Close is an exit code, not a new channel.** `quit()` exits 0; the launcher
reads a clean exit as deliberate and stops everything it started, while a
non-zero exit is a crash and gets restarted. Adopted components are left
running, which is what the tray tooltip says before it happens.

Two new controls in the overlay's button bar (design in T5.1): minimise to tray,
and close. Close asks the supervisor to stop everything and exits; it does not
merely hide the window.

**Verify:** close leaves no `kotodex`, no capture daemon, no read-stats — unless
read-stats was adopted rather than started, in which case it is left running.
That distinction is deliberate and gets a line in the tray menu tooltip.
**Commit:** `overlay: close and minimise`.

### T4.7 — Desktop entry and icon

**Status:** written, not installed. `kotodex/kotodex.svg` exported to 48–512
PNG, `kotodex.desktop` (validated by `desktop-file-validate`), and
`kotodex/install-entry.sh` to put both under `~/.local` along with symlinks for
`kotodex` and `kotodex-capture`. `--uninstall` removes exactly those and says
the databases were untouched.

Not run: installing puts an entry in your menu and launching it opens a window,
so that is yours to do. One command: `kotodex/install-entry.sh`.

Icon: a simple SVG, exported to 48/64/128/256/512 PNG into hicolor. Desktop
entry with `Name`, `Comment`, `Icon=kotodex`, `Exec=kotodex`, `Categories=Education;Languages;`,
`StartupWMClass` set so the tray/window associates correctly.

**Verify:** entry appears in the application menu after
`update-desktop-database`; launching from it works with no terminal.
**Commit:** `kotodex: desktop entry and icon`.

### T4.8 — X11 overlay backend

**Status:** required, per T0.4 — this is what makes GNOME work, so it is on the
critical path rather than a maybe. The backend must force `QT_QPA_PLATFORM=xcb`
for itself: inheriting the session default gives a native Wayland surface where
`WindowStaysOnTopHint` does nothing.

Verify click-through with XShape on GNOME as part of this task; T0.4 left it
unchecked.

Only if T0.4 says it is needed and viable. `layer-overlay/` gains a second
backend selected at startup: layer-shell where available, X11
`_NET_WM_STATE_ABOVE` + XShape input region otherwise. The Qt/QML/WebEngine
half, the web channel and the xdotool geometry tracking are shared — they are
already X11-based.

Print which backend was chosen at startup and in the doctor.

**Verify:** the T0.4 test matrix, re-run against the real overlay.
**Commit:** `layer-overlay: X11 backend`.

---

# Phase 5 — Overlay UX

### T5.1 — Button bar rework

Today: six always-visible buttons (`ℹ 👁 あ ⤢ ⚙ ⏸`) in one row at top-left,
with close and minimise still to add. Nine buttons in a row is too many over a
game.

Design: one small draggable handle. Click expands the bar; it collapses on
click-away or after a timeout. Grouping:

- always visible when collapsed: the handle, and the pause state if paused
- expanded: explain, hide line, ghost, scrollback, settings, pause, mobile,
  minimise, close

Keyboard shortcuts for the frequent ones, listed in settings.

**Verify:** every existing action still reachable; the bar does not cover the
line at any scale. **Commit:** `overlay: collapsible button bar`.

### T5.2 — Settings panel: tabs and new settings

Today: size, line height, spacing, backdrop, font, reset. Restructure into tabs
and add what is currently env-only or not configurable at all:

- **Type** — size, line height, spacing, font, weight, colour, shadow on/off
- **Placement** — strip height (`VN_OVERLAY_HEIGHT`), alignment fractions
  (`--text-x`, `--text-y`, `--text-size`, `--text-chars`) with a live drag
  mode, mobile scale, popup scale
- **Marks** — status highlighting on/off (T3.3), which statuses are painted,
  the common-word underline threshold, tint strength, ghost mode default
- **Behaviour** — explain on/off, lookup recording on/off, click vs hover to
  define, side-button actions (`SIDE_ACTIONS`), scrollback size
- **Reset** per tab and for everything

**Verify:** each setting changes the overlay live and survives a reload.
**Commit:** `overlay: settings tabs and new controls`.

### T5.3 — Font list from the system

`overlay.html` hardcodes eight font chips including `DNP Shuei Mincho Pr6`,
`HGSMinchoB` and `kaikoku PM` — fonts on one machine. A stranger sees chips that
do nothing.

Enumerate Japanese-capable installed fonts (`fc-list :lang=ja family`) via a new
read-stats endpoint, show those, and keep "As launched" first.

**Verify:** chips match `fc-list` output; selecting each changes the line.
**Commit:** `overlay: font list from installed fonts`.

### T5.4 — Scrollback panel

New panel over the layer surface: the last N lines, scrollable, each line
clickable for definitions exactly as the live line is. Data source already
exists — `GET /api/lines/before` (`read-stats/src/routes/reader/lines.rs`) —
so this is client work plus paging.

Requirements: opens over the whole screen (input region must expand, see T5.5),
closes on Escape and on click-away, remembers scroll position while open,
does **not** record a lookup differently from the live line, and shows the same
status marks. Jump-to-latest control. Optional: a search box over the session's
lines.

**Verify:** open mid-session, scroll back 200 lines, click a word, mine from it,
close — live feed resumes with no missed lines. **Commit:** `overlay: line scrollback`.

### T5.5 — Input region for full-screen panels

The layer surface only takes clicks where the page has drawn. Scrollback and an
expanded settings panel cover most of the screen, so the input region has to
grow while they are open and shrink again after — otherwise the game stops
receiving clicks.

**Verify:** with scrollback closed, clicks pass through everywhere except the
line box and the bar; with it open, they do not reach the game; after closing,
they do again. **Commit:** `overlay: input region follows open panels`.

### T5.6 — Settings storage

Today: `localStorage` keys `vn-overlay-type`, `vn-overlay-ghost`,
`vn-overlay-offset*`. Placement stays local (it is per screen). Everything the
installer, doctor or `#read` needs to agree on — status highlighting, explain
on/off, capture source — moves to read-stats `settings`, with an export/import
of the whole set for moving machines.

**Verify:** change a shared setting in the overlay, reload `#read`, it agrees.
**Commit:** `overlay: shared settings server-side`.

### T5.7 — Line source selection

Add clipboard and direct-WebSocket producers beside `vn-ws-logger.py`, all
writing the same `lines` rows. Selected in settings and reported by the
capability probe. The existing WS logger stays the default and keeps its
`clean_line()` filtering and dedup; the clipboard watcher reuses the same
filters.

Textractor's WS plugin crashes on abortive disconnect, so the direct-WS option
carries a warning and a clean-close implementation.

**Verify:** each source in turn produces lines in the overlay; switching sources
does not duplicate rows. **Commit:** `capture: clipboard and websocket sources`.

---

# Phase 6 — Installer

### T6.1 — `setup.sh` skeleton

Idempotent, re-runnable, no-op when everything is already set up. Structure:
detect platform → check dependencies → download models → dictionaries → Anki →
optional extras → install entry → print doctor output.

Every step prints what it is about to do and can be skipped. `--yes` accepts
defaults, `--dry-run` prints without acting.

**Verify:** `--dry-run` on the live machine changes nothing.
**Commit:** `setup: skeleton and platform detection`.

### T6.2 — Dependency check

Uses T3.5. Required: `curl`, `jq`, `ffmpeg`, `pactl`, `python3`, and the Qt
stack (`PySide6`, `qt6-webengine`, `layer-shell-qt` as **system** packages — a
venv build of PySide6 has no `org.kde.layershell`). Optional: `xdotool`, a
screenshot tool, `docker`.

Missing required → print one paste-able install command and stop. Missing
optional → note it and continue.

**Verify:** in a container with nothing installed, the printed command is
correct for that distro. **Commit:** `setup: dependency check`.

### T6.3 — Models

- SudachiDict **full** (`sudachi-dictionary-latest-full.zip`, ~127 MB,
  Apache-2.0) from the WorksApplications S3.
- silero VAD (`silero_vad.onnx`, 2.2 MB, MIT) from the silero-vad repo.

Both are dependencies, not prompts. Show progress, verify size, skip if present.

**Verify:** on a machine with neither, both land in the right place and
`kotodex doctor` goes green for them. **Commit:** `setup: sudachi and vad models`.

### T6.4 — Dictionaries

One prompt: *download the free dictionaries?* — Jitendex (CC-BY-SA, GitHub
releases, resolve the latest tag via the API), a frequency list, and a free
pitch dictionary. Import each with `jp-dict import`, assign roles via T1.7.

Then print, always: how to add a copyrighted dictionary (drop the zip in
`dictionaries/` and re-run), naming the ones that improve the experience most.

**Verify:** on an empty `knowledge.db`, accepting gives a working popup with
definitions, one frequency pill and pitch. **Commit:** `setup: free dictionary download`.

### T6.5 — Anki

Probe AnkiConnect. Present → run T2.5's note type check and offer to create
Lapis. Absent → explain that cards are off until Anki with AnkiConnect is
running, and that re-running `setup.sh` will pick it up.

**Verify:** all three cases (no Anki, Anki without Lapis, Anki with Lapis).
**Commit:** `setup: anki configuration`.

### T6.6 — Optional extras

Anthropic key prompt (explain), written to the config file with 600
permissions. Whisper: **not configured in this release** — one line saying what
it would add and pointing at `whisper-service/README.md`.

**Verify:** key accepted → explain button appears; skipped → it does not.
**Commit:** `setup: optional extras`.

### T6.7 — Application entry

Install the binaries into `~/.local/bin`, the icon into hicolor, the desktop
entry, and run `update-desktop-database`. Warn if `~/.local/bin` is not on
`PATH` and print the line to add.

**Verify:** on a clean VM, the entry appears and launches.
**Commit:** `setup: install application entry`.

### T6.8 — Uninstall

`setup.sh --uninstall`: removes the entry, icons, binaries, systemd unit if any,
and **asks separately** before touching `~/.local/share/jp-tools` — the
databases hold the reading history and must never go without an explicit yes.

**Verify:** uninstall then reinstall; data survives.
**Commit:** `setup: uninstall`.

---

# Phase 7 — Artifact and repository

### T7.1 — Build the artifact

`scripts/build-release.sh`: release-builds the Rust binaries, collects the
Python and web assets, `setup.sh`, `docs/`, licences and third-party notices
into `kotodex-<version>-linux-x86_64.tar.gz`. Prints the size and a checksum.

Decide here whether the tarball ships prebuilt binaries (bigger, no Rust
toolchain needed — strongly preferred for the target user) or builds on the
machine.

**Verify:** extract into an empty directory on a machine without the repo and
run `setup.sh`. **Commit:** `scripts: release artifact`.

### T7.2 — README rewrite

The root `README.md` currently opens with "Monorepo for Japanese language
learning tools" and a project list, and is stale (it places the overlay in
`vn-mine/`; it has been `read-stats/overlay/` since the layer-overlay split).

New structure: what it is in one sentence → the recording → what you get
(overlay, dictionary popup, Anki cards with the voiceline, optional reading
stats) → requirements including the compositor sentence from T0.4 → install in
three lines → configuration → what degrades and how → "also in this repo" for
yt-mine, manga-mine and the rest.

Keep the repo named `jp-tools`; name the release after the product. Add GitHub
topics: `japanese`, `visual-novel`, `anki`, `sentence-mining`, `texthooker`,
`linux`, `wayland`.

**Verify:** read it as someone who has never seen the project.
**Commit:** `README: lead with the product`.

### T7.3 — Screenshots and recording

- One screen recording (wf-recorder or OBS → GIF via ffmpeg palettegen, or MP4
  in the README) showing: line appears over the game → click a word → popup →
  mine → card in Anki with audio playing.
- Stills: overlay over a game, popup with definitions, scrollback, settings,
  doctor output, the dashboard.

**Use a freely-distributable VN for the demo.** Recording a commercial title's
art into a public README is a licensing question you do not want; pick a free
or open-licensed VN and say which it is.

**Verify:** media renders on GitHub, under 10 MB each.
**Commit:** `docs: screenshots and demo recording`.

### T7.4 — Repository hygiene

`LICENSE` (choose one), `CONTRIBUTING.md`, issue templates that ask for
`kotodex doctor` output, `THIRD-PARTY.md` listing SudachiDict, silero-vad,
Jitendex, Lapis and their licences, and a `SECURITY`/scope note that the
AnkiConnect proxy binds locally.

**Verify:** licence headers consistent; every bundled asset attributed.
**Commit:** `docs: licences and contribution guide`.

### T7.5 — Release

Tag, build, attach the tarball and checksum, write release notes from this plan.
Optionally a CI workflow that builds the artifact on tag.

**Verify:** download the release on a clean machine and follow the README
exactly. **Commit:** `ci: release workflow`.

---

# Phase 8 — Clean-machine validation

Run **after** Phase 7, on the actual artifact, not the repo.

### T8.1 — Ubuntu GNOME VM

Fresh Ubuntu (current LTS), GNOME Wayland. Download the tarball, run `setup.sh`,
accept defaults. Expect: dependency command correct for apt, models download,
dictionaries import, entry appears, overlay launches with whichever backend
T0.4 chose, doctor accurate about what is missing.

**Record every place the script was unclear** — that list is the next round of
work.

### T8.2 — Second distro

Fedora or openSUSE, KDE. Same pass, checking the dnf/zypper package names.

### T8.3 — No-dictionary path

Decline the dictionary prompt. The overlay must still start, show lines, and say
plainly that the popup needs dictionaries and how to get them.

### T8.4 — No-Anki path

Anki not installed. Mining controls absent, everything else works, doctor says
why.

### T8.5 — Fresh Anki profile

New profile, Lapis installed by the setup script, mine a card end to end
including audio and screenshot.

### T8.6 — Re-run and idempotence

Run `setup.sh` twice more. Nothing duplicated, no second dictionary rows
(`source_path` is the cache key and a moved zip is repointed, not re-imported),
no second desktop entry, no second read-stats.

---

# Task order summary

Phase 0 blocks everything (T0.4 blocks the GNOME promise and T4.8).
Phase 1 blocks Phase 2. Phase 3 needs Phase 1 for the dictionary rows.
Phase 4 blocks T4.6 and the installer's entry step. Phase 5 needs Phase 4 for
close/minimise. Phase 6 needs 1–5. Phase 7 needs 6. Phase 8 needs 7.

Parallelisable: T0.3 and T0.4 alongside Phase 1; T5.4 (scrollback) alongside
Phase 4; T7.3 (media) as soon as Phase 5 looks final.

# Open decisions

1. ~~**Product name** (T0.1)~~ — decided: Kotodex / コトデックス.
2. ~~**Compositor support statement** (T0.4)~~ — decided: layer-shell where the
   compositor has it, X11 always-on-top otherwise. GNOME included.
3. ~~**Systemd unit or supervisor child**~~ (T4.2) — decided: both. The unit
   stays; the supervisor adopts a running daemon and starts one only if there
   is none.
4. **Vendor the Lapis note type or link to it** (T2.5).
5. **Prebuilt binaries in the tarball or build on install** (T7.1).
6. **Licence** (T7.4).
7. **Which VN to record** for the README (T7.3).
8. ~~**Which frequency dictionary is the reader's**~~ (T1.3) — decided: the
   first by `dictionaries.priority`.
