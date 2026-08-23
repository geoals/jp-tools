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

## Left over from finished phases

- **T4.8 — X11 backend on GNOME.** Working here, unverified there. Log into
  GNOME Wayland, run the overlay over a fullscreen game, and check it stays
  above and that clicks land through it. KDE cannot test the stacking half:
  KWin puts an active fullscreen window above keep-above ones.

---

# Phase 2 remainder — Anki setup

### T2.5 — Note type check and creation

**Status:** blocked on open decision 1 (vendor Lapis or link to it) and on a
fresh Anki profile to verify against.

New: `kotodex anki check` (also called by the doctor and the installer). Probes
AnkiConnect, then `modelNames` / `modelFieldNames`, and reports one of:

- note type present with every configured field → ok
- present, fields missing → list exactly which, and the field map to fix
- absent → offer creation from a bundled Lapis definition, or point at the
  Lapis release deck

Bundle the Lapis note type as `assets/lapis/` (templates + CSS) with its
licence, or link to the upstream release deck. Vendoring is friendlier and needs
an update path; linking is zero maintenance and one more manual step.

**Verify:** against a fresh Anki profile with no Lapis: reports absent, creates
it, second run reports ok. **Commit:** `anki: note type check and setup`.

### T2.6 — Live card round-trip

**Status:** blocked — manual, needs a reading session and a fresh profile. This
is where the `JP_TOOLS_ANKI_FIELD_*` map gets its first real mine.

Mine one card in each style into a scratch deck, and look at it: glossary
renders, definitions page, pitch shows, image and audio attach, frequency sorts.

**Verify:** manual, screenshots into `docs/`. **Commit:** none (validation).

---

# Phase 5 remainder — Overlay UX

### T5.4 — Scrollback panel

New panel over the layer surface: the last N lines, scrollable, each line
clickable for definitions exactly as the live line is. Data source already
exists — `GET /api/lines/before` (`read-stats/src/routes/reader/lines.rs`) — so
this is client work plus paging.

Requirements: opens over the whole screen (input region must expand, see T5.5),
closes on Escape and on click-away, remembers scroll position while open, does
**not** record a lookup differently from the live line, and shows the same
status marks. Jump-to-latest control. Optional: a search box over the session's
lines.

Its size control is the one setting the deferred Behaviour tab is waiting on.

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

Placement stays in `localStorage` (it is per screen). Everything the installer,
doctor or `#read` needs to agree on — status highlighting, explain on/off,
capture source — moves to read-stats `settings`, with an export/import of the
whole set for moving machines.

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

`setup.sh` covers the skeleton, dependency check, models, extras, application
entry and uninstall. What is left:

### T6.4 — Dictionaries

**Status:** blocked on which dictionaries to ship. `setup.sh` imports whatever
is in `dictionaries/` and prints how to add more, but downloads nothing —
Jitendex publishes no GitHub release assets, so its zip needs a URL that is not
derivable from the API, and the frequency and pitch picks are a licensing and
taste call.

Decide the three, then: download each, `jp-dict import` it (roles are guessed),
and keep the printed advice for the copyrighted ones that cannot be shipped.

**Verify:** on an empty `knowledge.db`, accepting gives a working popup with
definitions, one frequency pill and pitch. **Commit:** `setup: free dictionary download`.

### T6.5 — Anki note type

**Status:** half. `setup.sh` probes AnkiConnect and says plainly that mining is
off until it answers. The note type check and Lapis creation wait on T2.5.

**Verify:** all three cases (no Anki, Anki without Lapis, Anki with Lapis).
**Commit:** `setup: anki note type`.

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

The root `README.md` opens with "Monorepo for Japanese language learning tools"
and a project list, and is stale — it still places the overlay in `vn-mine/`.

New structure: what it is in one sentence → the recording → what you get
(overlay, dictionary popup, Anki cards with the voiceline, optional reading
stats) → requirements including the compositor sentence → install in three lines
→ configuration → what degrades and how → "also in this repo" for yt-mine,
manga-mine and the rest.

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

# Task order

T2.5 gates T6.5. T5.4 needs T5.5. Phase 7 needs 6; Phase 8 needs 7. T7.3
(media) can start as soon as Phase 5 looks final.

# Open decisions

1. **Vendor the Lapis note type or link to it** (T2.5).
2. **Prebuilt binaries in the tarball or build on install** (T7.1).
3. **Licence** (T7.4).
4. **Which VN to record** for the README (T7.3).
5. **Which dictionaries `setup.sh` downloads** (T6.4) — a bilingual, a frequency
   list and a pitch dictionary, all freely redistributable.
