# Release plan — a shareable Linux VN reading overlay

Product name: **Kotodex / コトデックス**. `kotodex` is the binary, desktop-entry
id and tray name. The crate, service and database names (`read-stats`,
`read-stats.db`) do not change.

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

Non-goals: whisper auto-setup, Windows/macOS, X11-only compositors older than
the ones in `docs/compositors.md`.

## Ground rules

- **Commit straight to `master`**, one commit per completed task, only when its
  verification passes.
- **Never restart the live stack while a VN is being read.** Use
  `scripts/dev-instance.sh` (port 3299, copy of the data) for anything
  read-stats-side. Overlay/static changes need `vn-overlay.sh restart`.
- **The golden corpus is the regression gate** for anything touching
  dictionaries, roles or the tokenizer:

  ```
  KOTODEX_SUDACHI_DICT_PATH=$PWD/system_full.dic \
    cargo test -p jp-core --features test-support -- --ignored
  ```

  The failure diff is the review — read it, and only then regenerate with
  `cargo run --release --example golden -p jp-core --features test-support -- <knowledge.db> jp-core/tests/golden/corpus.txt $PWD/system_full.dic`.
- A task that turns out bigger than its description gets split rather than
  stretched.
- **This document is the live status.** A finished task leaves this file and
  keeps only whatever fact the remaining work needs.

---

# What the remaining work builds on

- **Docs.** `docs/degradation.md` is the specification the installer and doctor
  both implement. `docs/compositors.md` holds the compositor matrix.
  `docs/release-notes.md` is the text for the first release.
- **Compositors.** Layer-shell where the compositor has it, X11 always-on-top
  otherwise. On GNOME the Qt process must run under `QT_QPA_PLATFORM=xcb` — a
  native Wayland surface silently ignores the on-top hint.
  `layer-overlay/backend.py` picks the backend before Qt starts;
  `LAYER_OVERLAY_BACKEND` forces one.
- **The install root** is `jp_core::install::install_root()`: `KOTODEX_ROOT`,
  else the binary's own location (`<root>/target/release/<bin>`, checked by
  looking for the assets), else the build workspace. The compiled-in path is
  last because for a release it names a CI container.
- **One Anki field map.** `jp_mine_core::config::AnkiConfig` — Lapis by default,
  every name overridable through `KOTODEX_ANKI_FIELD_*`. read-stats' config,
  the overlay's mine route and `vn-capture.sh` all read it; nothing spells a
  field name or the note type for itself. `KOTODEX_ANKI_STYLE` = `lapis`
  (default) | `legacy` picks the card markup.
- **Capabilities.** `read-stats/src/routes/reader/capabilities.rs` is the one
  probe, served under `capabilities` on `/api/reader/state`; every row is
  `{ok, detail, fix}`. The overlay draws only controls that can work.
- **Platform.** `scripts/lib/platform.sh` maps distro → package manager and
  carries per-distro package names. `scripts/kotodex-doctor.sh` is the doctor,
  and the installer's closing summary calls it rather than reimplementing it.
- **Installer.** `setup.sh` — re-runnable, `--dry-run`, `--yes`, `--uninstall`.
  The API key goes in `.env`, the file every crate already loads through
  dotenvy.
- **Launcher.** `kotodex/kotodex.py` adopts-or-starts capture, read-stats and
  the overlay, single-instances on a `QLocalServer` socket and owns the tray.
- **Release build.** `scripts/build-release.sh [version]` makes the tarball;
  `.github/workflows/release.yml` runs it in `rust:1-bookworm` on a `v*` tag,
  gates the glibc floor at 2.35 and attaches the result to a **draft** release.

Two constraints for anything new drawn in the overlay:

- **A stored setting that changes shape is dropped, not merged.** The shell
  keeps localStorage across releases, and one mistyped value took the whole
  module's setup down — including `report()`, which left the overlay
  unclickable.
- **QtWebEngine is the surface, and it is not this machine's Chromium.** A flex
  column shrinks its rows instead of scrolling there. Check anything new in a
  real view, not only in a browser.

## Naming

**Kotodex, a reading log for Japanese.** The repo is `geoals/kotodex` and the
data lives in `~/.local/share/kotodex`. The crates keep their own names.
yt-mine, manga-mine and the services stay in the repo — they share `jp-core`
and `jp-mine-core`, so a split would mean publishing those.

Held elsewhere by dormant projects with no traction: the GitHub user `kotodex`
and PyPI `kotodex`. `kotodex.com` is ours, and `com.kotodex.Kotodex` is the app
id to use for the desktop entry, the icon and a future Flatpak.

## This machine

- **The launcher owns the capture daemon.** `vn-buffer.service` is stopped and
  disabled. `kotodex restart` and the tray's "Restart everything" are what make
  new code live — relaunching the app does not, because a component it adopted
  is one it never touches.
- **whisper-service is started by hand** (`scripts/start-all.sh whisper`). Its
  container has no restart policy, so it does not survive a reboot. Decided
  against automating it; the options are `restart: unless-stopped` in the
  compose file, or a fourth launcher child.
- **`.env` pins the personal note type and every legacy field name**, so the
  live export is unchanged while the defaults everyone else gets are Lapis.
  A new field added to `AnkiConfig` needs its pin added here too, or it silently
  takes the Lapis name.

---

# What is left

## Decide

### Which VN to record for the README (T7.3)

Must be freely distributable — a commercial title's art in a public README is a
licensing question worth avoiding.

## Build

### T7.3 — Screenshots and recording

- One screen recording (wf-recorder or OBS → GIF via ffmpeg palettegen, or MP4)
  showing: line appears over the game → click a word → popup → mine → card in
  Anki with audio playing.
- Stills: overlay over a game, popup with definitions, scrollback, settings,
  doctor output, the dashboard.

**Verify:** renders on GitHub, under 10 MB each.
**Commit:** `docs: screenshots and demo recording`.

### T7.5 — Tag the release

`git tag v<version> && git push origin v<version>`, then publish the draft the
workflow creates. Afterwards: the GitHub topics, once the repo is public —
`gh repo edit --add-topic japanese,visual-novel,anki,sentence-mining,texthooker,linux,wayland`.

## Verify by hand

1. **A card through the overlay's mine, on the Lapis defaults.** Switch Anki to
   a profile with Lapis, run read-stats from a directory with no `.env` so the
   shipped defaults apply, and mine one. Check the fields land, then do the same
   under `.env` for the legacy note type.
2. **The overlay on GNOME Wayland** over a fullscreen game: does it stay above,
   do clicks land through it. KDE cannot test the stacking half — KWin puts an
   active fullscreen window above keep-above ones. Better tested by logging into
   a GNOME session on this machine than in a VM.
3. **`setup.sh --uninstall` on a machine that is not this one.** It asks for a
   typed `DELETE` before touching the reading history.
4. **`anki-setup install-lapis` on an older Anki**, where `importPackage` still
   works and the `guiImportFile` fallback is not the path taken.
5. **A clean-machine pass on the real artifact**, not the repo — the desktop
   half of it. The headless half (dependency detection, both model downloads,
   both dictionary downloads and their roles, the desktop entry, the doctor,
   a second run, `--dry-run`, `--uninstall`) passes in an Ubuntu container, and
   `scripts/build-release.sh` in `rust:1-bookworm` is how to reproduce a
   release-identical tarball to test with.

