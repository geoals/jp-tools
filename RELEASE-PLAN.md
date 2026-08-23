# Release plan — a shareable Linux VN reading overlay

Product name: **Kotodex / コトデックス**. `kotodex` is the binary, desktop-entry
id and tray name. The crate, service and database names (`read-stats`,
`read-stats.db`) do not change.

| thing | value |
|---|---|
| launcher binary | `kotodex` |
| desktop entry | `kotodex.desktop`, `Name=Kotodex` |
| icon | `kotodex` in hicolor 48–512 + scalable SVG |
| capture daemon | `kotodex-capture` |
| install prefix | `~/.local/share/kotodex` (data), `~/.local/bin` (binaries) |
| config file | `.env` in the install directory, mode 600 |

## What "done" means

On ~90% of current desktop Linux installs:

1. Download one tarball from a GitHub release.
2. Run `./setup.sh`, answer a handful of yes/no prompts.
3. Get an application entry with a name and an icon.
4. Launch it → the capture daemon, the read-stats server and the overlay all
   come up. Launch it again → it detects the running instance and does nothing.
5. The tray owns show/hide and quit.
6. Anything missing degrades to a smaller working product with one clear
   sentence saying what is off and the one command that turns it on.

Non-goals for this release: whisper auto-setup, Windows/macOS, X11-only
compositors older than the ones in `docs/compositors.md`.

## Ground rules

- **Commit straight to `master`**, one commit per completed task, only when its
  verification passes.
- **Never restart the live stack while a VN is being read.** Use
  `scripts/dev-instance.sh` (port 3299, copy of the data) for anything
  read-stats-side. Overlay/static changes need `vn-overlay.sh restart`.
- **The golden corpus is the regression gate** for anything touching
  dictionaries, roles or the tokenizer:

  ```
  JP_TOOLS_SUDACHI_DICT_PATH=$PWD/system_full.dic \
    cargo test -p jp-core --features test-support -- --ignored
  ```

  The failure diff is the review — read it, and only then regenerate with
  `cargo run --release --example golden -p jp-core --features test-support -- <knowledge.db> jp-core/tests/golden/corpus.txt $PWD/system_full.dic`.
- Prose in code and docs follows the repo's rules: plain English, say why not
  what, no history in comments.
- A task that turns out bigger than its description gets split rather than
  stretched.
- **This document is the live status.** A task carries a status line only when
  it is blocked or partly done; finished tasks leave this file and keep only
  whatever fact the remaining work needs.

---

# What is already in place

Phases 0–3 are complete, along with the process model (Phase 4) and the overlay
bar, settings panel and font list (T5.1–T5.3). What the remaining work has to
build on:

- **Docs.** `docs/degradation.md` is the specification the installer and doctor
  both implement. `docs/compositors.md` holds the compositor matrix.
- **Compositors.** Layer-shell where the compositor has it, X11 always-on-top
  otherwise, GNOME included. On GNOME the Qt process must run under
  `QT_QPA_PLATFORM=xcb` — a native Wayland surface silently ignores the on-top
  hint. `layer-overlay/backend.py` picks the backend before Qt starts;
  `LAYER_OVERLAY_BACKEND` forces one.
- **Dictionary roles.** No dictionary title appears in code except as an
  installer default. `Frequency` and `Pitch` are roles; the master is optional
  and falls back; `jp-dict import` guesses a role from what the zip holds and
  `--role` overrides. Only a new row is guessed at, so a `set-role` survives a
  sync.
- **Cards.** `JP_TOOLS_ANKI_STYLE` = `lapis` (default) | `legacy`; both are
  supported profiles, not a migration. `AnkiConfig::default()` is the Lapis
  field map, and every name is overridable through `JP_TOOLS_ANKI_FIELD_*`,
  which `vn-capture.sh` reads too. The live setup pins legacy in a gitignored
  `.env`.
- **Capabilities.** `read-stats/src/routes/reader/capabilities.rs` is the one
  probe, served under `capabilities` on `/api/reader/state`; every row is
  `{ok, detail, fix}`. The overlay draws only controls that can work.
- **Platform.** `scripts/lib/platform.sh` maps distro → package manager and
  carries per-distro package names. `scripts/kotodex-doctor.sh` is the doctor,
  and the installer's closing summary must call it rather than reimplement it.
- **Relocation.** `jp_core::install::install_root()` — `KOTODEX_ROOT`, else the
  workspace the binary was built in. Every asset path resolves through it.
- **Installer.** `setup.sh` — re-runnable, `--dry-run`, `--yes`, `--uninstall`.
  It checks dependencies against `platform.sh`, downloads SudachiDict and the
  silero VAD model, imports whatever is in `dictionaries/`, probes AnkiConnect,
  stores an Anthropic key, calls `kotodex/install-entry.sh` and ends by running
  the doctor. **The API key goes in `.env`, not `config.toml`** — that is the
  file every crate already loads through dotenvy, and a second config format
  would need plumbing in five places to buy nothing.
- **Launcher.** `kotodex/kotodex.py` adopts-or-starts capture, read-stats and
  the overlay, single-instances on a `QLocalServer` socket and owns the tray.
  The overlay is a child and is deliberately unsupervised — the tray's Quit is
  the only deliberate stop. `kotodex/install-entry.sh` installs the desktop
  entry, icons and `~/.local/bin` symlinks, and `--uninstall` removes exactly
  those.
- **Overlay UI.** The bar is `☰ ℹ 👁 ⚙`, dragged by the handle; everything else
  moved into a three-tab settings panel. Two constraints for anything new drawn
  there:
  - **A stored setting that changes shape is dropped, not merged.** The shell
    keeps localStorage across releases, and one mistyped value took the whole
    module's setup down — including `report()`, which left the overlay
    unclickable.
  - **QtWebEngine is the surface, and it is not this machine's Chromium.**
    A flex column shrinks its rows instead of scrolling there. Check anything
    new in a real view, not only in a browser.

## This machine

- **The launcher owns the capture daemon.** `vn-buffer.service` is stopped and
  disabled. `kotodex restart` and the tray's "Restart everything" are what make
  new code live — relaunching the app does not, because a component it adopted
  is one it never touches.
- Capture is currently *adopted* rather than owned, from being started by hand.
  One clean quit-and-relaunch of Kotodex fixes that; nothing is wrong meanwhile.
- **whisper-service is started by hand** (`scripts/start-all.sh whisper`). Its
  container has no restart policy, so it does not survive a reboot. Decided
  against automating it for now; the options are `restart: unless-stopped` in
  the compose file, or a fourth launcher child.
- **`.env` pins the personal note type**, so the live export is unchanged while
  the defaults everyone else gets are Lapis.

## Left over from finished phases

- **T4.8 — X11 backend on GNOME.** Working here, unverified there. Log into
  GNOME Wayland, run the overlay over a fullscreen game, and check it stays
  above and that clicks land through it. KDE cannot test the stacking half:
  KWin puts an active fullscreen window above keep-above ones.

---

# Phase 2 remainder — Anki setup

### T2.5 — Note type check and creation

**Status:** done. `anki-setup` (in jp-mine-core, because that is where the field
map is) reports one of: AnkiConnect not answering, note type absent, fields
missing — naming which, listing what the note type does have, and the
`JP_TOOLS_ANKI_FIELD_*` line to rename each — or ok. `kotodex anki check` and
`setup.sh` both call it, and `setup.sh` offers `install-lapis` when the missing
note type is Lapis.

**Lapis is downloaded, not vendored.** It is GPL-3.0, it moves, and its release
is an `.apkg` Anki can import — so a copy here would be a second version to keep
current for nothing. The release asset is resolved at run time.

Two things a fresh profile found, both real:

- **`/tmp` is not a path Anki can read.** A Flatpak Anki has its own, and the
  import fails with a file-not-found naming a path that plainly exists. The
  `.apkg` goes in Anki's profile directory now, found through
  `getMediaDirPath`.
- **`importPackage` is gone in current Anki** (26.05 here). The importer
  AnkiConnect calls was replaced, and the refusal comes back as an exception
  with an empty message — not relative to `collection.media`, not an absolute
  path, not anywhere. So it is tried first, for the older versions where it
  still works silently, and `guiImportFile` is the fallback: Anki's own dialog,
  opened on the file, one click.

Verified end to end against a fresh profile: absent → download → dialog →
imported, and `check` then reports Lapis with **every one of the thirteen
configured fields present**, which is the Lapis defaults in `AnkiConfig`
confirmed against the real note type.

### T2.6 — Live card round-trip

**Status:** blocked — manual, needs a reading session. The fresh profile with
Lapis on it exists; what is missing is a card going into it.

Mine one card in each style into a scratch deck, and look at it: glossary
renders, definitions page, pitch shows, image and audio attach, frequency sorts.

**Verify:** manual, screenshots into `docs/`. **Commit:** none (validation).

---

# Phase 5 remainder — Overlay UX

### T5.4 — Scrollback panel

**Status:** done. ▤ opens the last lines in a panel hanging under the bar —
inside `#explain-box`, so it is anchored to its own button and moves with the
drag — paged back a hundred at a time from the oldest id held — an offset would slide, since lines
keep arriving while it is open. Escape closes the popup first and then the
panel; a click on the backdrop closes it; a new line is appended only when the
view is already at the bottom.

The rows are built by `renderLine`, which is `draw`'s body extracted, and they
carry the *same* click, side-button and wheel handlers rather than copies. A
lookup from the scrollback is a lookup.

`/api/lines/before` gained `ruby`: a reading that appeared only while a line was
current would make the two surfaces disagree about what the game wrote.

Settled by use, after the first pass drew it fullscreen and set like the line:

- **Fixed type, smaller than the line** (1.7rem). The line is sized to register
  with the game's own text, which is larger than anything wants to read
  fourteen of at once, and nothing in the panel sits over artwork.
- **Width is read off `#box`**, not derived — the line box is sized by
  character count when aligned to the game and by viewport insets when not, and
  only one of those can be written in the stylesheet.
- **Session dividers**, split on `settings.session_gap_secs` so they mark the
  same sittings `#read` and `stats::derive_sessions` do. Recomputed from
  scratch on every change: a new line closes the sitting above it, and a page
  loaded above can reveal that the top of a sitting is the middle of one.
- **The three panels under the bar are alternatives.** One table maps each
  button to the panel it owns; a button owning none — hide, the handle — closes
  all three.
- **`place()` anchors to what was clicked.** It pinned every popup to the live
  line box, so a history word opened its definition at the bottom of the
  screen; and `#popup` precedes `#explain-box` with no z-index, so the panel
  painted over it.

### T5.5 — Input region for full-screen panels

**Status:** done. `report()` hands the shell the whole screen while the
scrollback is open and the measured rectangles otherwise. The settings panel
never needed it — it is drawn inside `#explain-box`, which is already in the
list.

### T5.6 — Settings storage

**Status:** deferred, and mostly overtaken. Everything two surfaces have to
agree on is server-side already — status painting, the underline rank, and now
the line source and its address. What is left is an export/import of the whole
set for moving machines, which is not wanted yet.

### T5.7 — Line source selection

**Status:** done, **untested against a live switch** — the daemon was left alone
because a session was being read. It needs `systemctl --user restart
vn-buffer.service` (the unit still carries the old name, by design) before the
new logger is running at all.

`settings.line_source` is `ws` or `clipboard`, `settings.line_source_ws_url` is
the address — Textractor's port is configurable there and was hardcoded here.
`vn-ws-logger.py` polls both, so a switch takes effect without a restart, the
same way pausing does; `VN_WS_URL` still wins, so a hand-started logger is not
moved by whatever the panel last saved.

**One producer, not two.** The clipboard is another source feeding the same
`emit()`, which is `read_lines`' body extracted. Everything after the source —
`clean_line`, the ruby split, the continuation join, the dedup, both sinks — is
what makes a captured string a line. Whatever is on the clipboard when the
watch starts is the previous value, never a line, or every switch would log the
last thing copied.

The clipboard reader is `wl-paste`, else `xclip`, else `xsel`, polled four
times a second. Nothing installed says so rather than failing quietly.

The direct-WebSocket producer from the original task is not built and is not
needed: pointing `line_source_ws_url` at another hooker is the same thing.

---

# Phase 6 — Installer

`setup.sh` covers the skeleton, dependency check, models, extras, application
entry and uninstall. What is left:

### T6.4 — Dictionaries

**Status:** done. `setup.sh` offers two, both resolved rather than pinned and
neither redistributed:

- **Jitendex** (CC BY-SA 4.0), from the latest release of
  `stephenmk/stephenmk.github.io` — not the `jitendex` repo, which publishes no
  assets.
- **the Jiten frequency list**, `GET https://api.jiten.moe/api/frequency-list/download`.
  It ranks the media people read, which is what the underline and the sweep
  order are supposed to mean. HEAD is refused there, so the size check on the
  result is the only probe.

Neither is offered when a dictionary of that kind is already imported — under
any filename, since `source_path` is the cache key and a second copy is a
duplicate row.

Verified on an empty database: Jiten lands as `frequency`, Jitendex as
`reference` and then takes `master` through T1.6's fallback.

A pitch dictionary is still left to the reader — the free ones need their
redistribution terms checked, and setup.sh says what is worth adding.

### T6.5 — Anki note type

**Status:** done with T2.5. `setup.sh` runs `anki-setup check`, prints its
report, and offers the Lapis import when that is what is missing.

---

# Phase 7 — Artifact and repository

### T7.1 — Build the artifact

**Status:** done. `scripts/build-release.sh [version]` →
`target/release-artifact/kotodex-<version>-linux-x86_64.tar.gz`, 11 MB, with a
sha256. **Prebuilt binaries**: the reader this is for runs visual novels, not
rustup.

Two things had to change first, both because a compiled-in path does not
survive being moved:

- **`jp_core::install::install_root()`** is where the assets are — the overlay
  page, the dashboard's static files, `backend.py`, `vn-capture.sh`,
  `dictionaries/`. `KOTODEX_ROOT` overrides it and `kotodex.py` sets it for
  every child; without it the build's own workspace is the answer, which is what
  a checkout wants.
- **`start-all.sh` skips the build when there is no cargo**, and defaults to the
  release profile then — a tarball has release binaries and no toolchain.

Binaries go in `target/release/` inside the tarball rather than `bin/`: it is
where setup.sh, kotodex.py and the doctor already look, and one layout for both
cases is one thing to be wrong about.

Verified: extracted somewhere else, with cargo off `PATH`, setup.sh reports
"shipped binaries" and read-stats serves the dashboard, the overlay page, the
vendored modules and the capability probe.

### T7.2 — README rewrite

**Status:** done, apart from the media. Leads with what Kotodex is, what you
get, the compositor requirement, three install lines, dictionaries, what
degrades, and "also in this repository" for yt-mine and manga-mine, which are
not in the release. The recording from T7.3 slots in under the first paragraph.

Repository stays `jp-tools`; the release is named after the product. Still to
do: GitHub topics — `japanese`, `visual-novel`, `anki`, `sentence-mining`,
`texthooker`, `linux`, `wayland`.

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

**Status:** half. `THIRD-PARTY.md` and `.github/ISSUE_TEMPLATE/` (the bug
template asks for `kotodex doctor` output) are in. The scope note about
loopback binding is the last section of `THIRD-PARTY.md`.

`LICENSE` is GPL-3.0. `CONTRIBUTING.md` is **not wanted** — it sets expectations
with strangers sending patches, and GPL-3.0 plus the issue template covers what
someone needs until PRs actually arrive.

Left: the GitHub topics, once the repo is public —
`gh repo edit --add-topic japanese,visual-novel,anki,sentence-mining,texthooker,linux,wayland`.

**Verify:** licence headers consistent; every bundled asset attributed.
**Commit:** `docs: licence and contribution guide`.

### T7.5 — Release

**Status:** half. `docs/release-notes.md` is written, and
`.github/workflows/release.yml` builds the tarball on a `v*` tag (or on
demand) and uploads it to the release — on ubuntu-22.04, so the shipped
binaries link a glibc old enough for the distros in `docs/compositors.md`.

Left: tag it, and check the download on a clean machine against the README.
That is T8.1's pass, on the release rather than the local build.

### T7.6 — The dashboard must not need a CDN

**Status:** done. preact, htm and `@preact/signals` are in
`web-shared/vendor/`, and all three `spa.html` import maps point there.
manga-mine now serves `/shared` like the other two, so the vendored copy is one
copy. Verified with `dev-instance.sh browser`: every view renders.

---

# Phase 8 — Clean-machine validation

Run **after** Phase 7, on the actual artifact, not the repo.

### T8.1 — Ubuntu GNOME VM

Fresh Ubuntu (current LTS), GNOME Wayland. Download the tarball, run `setup.sh`,
accept defaults. Expect: dependency command correct for apt, models download,
dictionaries import, entry appears, overlay launches on the X11 backend, doctor
accurate about what is missing. This also settles T4.8.

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

# What is left

**To build:** T5.6's settings export/import (deferred, not wanted yet) and
T7.3's screenshots and recording. Nothing else on this plan is unbuilt; T7.5 is
down to pushing a tag.

**To verify by hand** — the list is in the tasks above, gathered here:

1. **Mine a card into the real profile.** T2.6, and the last unverified step of
   the whole card path. Switch Anki back to `User 1` first — the fresh
   `kotodextest` profile has Lapis but not the pinned note type, and mining
   against it is what the "Anki: model was not found" line now reports.
2. **The overlay on GNOME Wayland** over a fullscreen game: does it stay above,
   do clicks land through it. T4.8, and the last thing holding the "90% of
   installs" claim. T8.1 settles it too.
3. **`setup.sh` on a clean VM.** Three paths have never run here because
   everything is already present: the SudachiDict download and unzip, the VAD
   download, and the Jitendex/Jiten prompts. Each URL and the unzip path were
   checked by hand, but not through the script's own flow.
4. **`setup.sh --uninstall`**, never run — it asks for a typed `DELETE` before
   touching the reading history, so it wants a machine that is not this one.
5. **`anki-setup install-lapis` on an older Anki**, where `importPackage` still
   works silently and the `guiImportFile` fallback is not the path taken.

# Open decisions

1. ~~**Vendor the Lapis note type or link to it**~~ (T2.5) — decided: neither.
   Download `Lapis.apkg` from the upstream release and import it through
   AnkiConnect's `importPackage`.
2. ~~**Prebuilt binaries in the tarball or build on install**~~ (T7.1) —
   decided: prebuilt.
3. ~~**Licence**~~ (T7.4) — decided: GPL-3.0.
4. **Which VN to record** for the README (T7.3).
5. ~~**Which dictionaries `setup.sh` downloads**~~ (T6.4) — decided: Jitendex
   and the Jiten frequency list.
