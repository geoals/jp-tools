# Kotodex on Windows

The core tier runs: the ledger, the dashboard, the dictionaries and the reader in
a browser. `setup.ps1` installs it. Capture and the overlay do not, and the
Textractor source does not, so **no text arrives on its own** — a source has to
post to `POST /api/lines` from somewhere.

## What was needed to get here

Less than expected. The Rust is portable — pure-Rust sudachi.rs, rustls, no
`libc`, migrations `include_str!`'d — so the first Windows build was clean with no
code changes at all. Every platform assumption sat in five files, and all of them
were a shell-out to a Linux CLI:

| was | is |
|---|---|
| four copies of `$HOME/.local/share/kotodex` | `jp_core::install::data_dir()`, `%LOCALAPPDATA%` off Linux |
| `fc-list :lang=ja` | `fontdb` + `ttf-parser` in process |
| `notify-send` | `notify-rust` |
| relative `system_full.dic` | resolved from `install_root()` |

The first three are wins on Linux independently: one fewer required tool, no
process spawn on a request path, and the font list stopped depending on
fontconfig being installed. That is the shape of the whole port — **stop shelling
out** — and Windows support is a side effect of it rather than the goal.

## What is left, in the order worth doing it

1. **A source.** Nothing else matters for using it: without one the feed is
   empty. `sources/textractor/vn-ws-logger.py` is 916 lines, and most of them are
   the cleaning rules — repeat collapsing, speaker stripping, continuation
   detection, ruby at UTF-16 offsets. `sources/README.md` says that cleaning is
   deliberately the source's own because none of it generalises, so it cannot be
   moved server-side to make the client thin. **Ship the same file frozen with
   PyInstaller** rather than porting it: two implementations of those rules would
   drift, and the file already has tests against it. One edit is needed, a
   Windows branch in `read_clipboard()`.
2. **Audio capture**, if anyone wants it. Two real blockers, not seven: ffmpeg has
   no WASAPI loopback device on Windows (`cpal` does support loopback, ~80 lines,
   and beats making the user install a virtual cable), and `vn-capture.sh` is 514
   lines of bash with 23 `curl`/`jq` calls. Rewriting that orchestration in Rust
   is worth doing for Linux too; `vn-vad.py` and `vn-trim.py` are portable as they
   are. Screenshots are *easier* here: `ffmpeg -f gdigrab -i title=` needs no
   `xdotool` and no ImageMagick.
3. **Packaging.** Inno Setup around a zip of the two exes plus `static/`,
   `overlay/`, `web-shared/`, `templates/`. Ship no dictionaries — licensing — and
   fetch SudachiDict on first run. A `windows-latest` CI job for repeatable
   artifacts; not needed for discovery, a VM answers that faster.
4. **The overlay**, last and optional. The page (~2,900 lines) is reusable as-is
   and `layer_overlay.py` is ~70% reusable; `xshape.py`, `xwatch.py` and
   `backend.py` are X11 to the core and get replaced by ~400 lines of `ctypes`
   (`WS_EX_LAYERED|TRANSPARENT|TOPMOST` + `SetWindowRgn`, a WinEvent hook for
   geometry, a named pipe instead of `SIGUSR1`). The `shell` contract in
   `layer-overlay/README.md` is already the seam. **Windows cannot overlay
   DirectX exclusive fullscreen at all** — borderless windowed is a documented
   requirement, not a bug, and most VN engines run windowed anyway.

## Decisions, so they are not relitigated

- **No `install-tier` file.** `setup.sh` writes one so the doctor does not report a
  missing overlay as a fault. The doctor is bash and cannot run here, so nothing
  would read it, and a file nothing reads is a file that drifts.
- **No separate doctor.** A part that cannot exist on this platform has no
  capability row at all — see `docs/degradation.md`. `kotodex-doctor.sh`'s `cap`
  already skips a key the server did not send, and a surface reading one draws
  nothing, so the fix was for the probe to stop claiming five Linux-only rows.
  A `fix` line naming a package that will never be installable reads as a broken
  install rather than a smaller one.
- **The revocation check is skipped only when it cannot be made.** `curl` exit 35
  triggers a retry with `--ssl-no-revoke` and says so. Passing it unconditionally
  weakened the check on every install to work around one network.
- **Jitendex resolves through `releases/latest/download`, not the API.**
  Unauthenticated `api.github.com` allows 60 requests an hour per address, and a
  machine behind a company NAT shares that count.

## Two Windows traps worth knowing

- **Windows PowerShell decodes a response body as ISO-8859-1** unless the server
  names a charset, and axum sends bare `application/json`. Every em dash in the
  server's own prose arrives as mojibake. `[Console]::OutputEncoding` does not fix
  it — the string is already wrong. `setup.ps1`'s `GetJson` reads the bytes.
- **A `.ps1` with no byte-order mark is read as the machine's ANSI codepage** by
  Windows PowerShell, so a non-ASCII character in a *string* mangles. Both scripts
  here stay ASCII rather than carrying a BOM.
