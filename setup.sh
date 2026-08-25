#!/usr/bin/env bash
# Set Kotodex up on this machine, and say what is still missing when it ends.
#
#   ./setup.sh [--yes] [--dry-run] [--uninstall] [--help]
#
# Re-runnable: every step checks before it acts, so a second run is a no-op and
# a run after installing something picks that up. Nothing needs root — the
# binaries, icon and desktop entry all go under ~/.local.
set -uo pipefail

# Bash's own expansion rather than dirname: a machine missing the basics is
# exactly the one this script exists for, and it must reach its own error
# message rather than die resolving its path.
HERE="${BASH_SOURCE[0]%/*}"
[ "$HERE" = "${BASH_SOURCE[0]}" ] && HERE="."
if [ -r "$HERE/scripts/lib/platform.sh" ]; then
  source "$HERE/scripts/lib/platform.sh"
else
  echo "setup.sh must be run from inside the Kotodex directory" >&2
  exit 1
fi

ASSUME_YES=0
DRY_RUN=0
UNINSTALL=0

DATA="$HOME/.local/share/kotodex"
ENV_FILE="$HERE/.env"

SUDACHI_URL="http://sudachi.s3-website-ap-northeast-1.amazonaws.com/sudachidict/sudachi-dictionary-latest-full.zip"
SUDACHI_MIN_BYTES=100000000
VAD_URL="https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx"
VAD_MIN_BYTES=1000000

bold=$'\033[1m'; red=$'\033[31m'; yellow=$'\033[33m'; green=$'\033[32m'; off=$'\033[0m'
[ -t 1 ] || { bold=""; red=""; yellow=""; green=""; off=""; }

step()  { printf '\n%s==> %s%s\n' "$bold" "$1" "$off"; }
say()   { printf '    %s\n' "$1"; }
good()  { printf '    %s✓%s %s\n' "$green" "$off" "$1"; }
skip()  { printf '    %s—%s %s\n' "$yellow" "$off" "$1"; }
fail()  { printf '    %s✗%s %s\n' "$red" "$off" "$1"; }
have()  { command -v "$1" >/dev/null 2>&1; }

usage() {
  sed -n '2,10p' "${BASH_SOURCE[0]}" | sed 's/^# \?//'
  exit 0
}

# Yes/no, defaulting to yes. `--yes` takes the default without asking; under
# `--dry-run` nothing is done either way, so the prompt is pointless.
confirm() {
  local prompt="$1" answer
  [ "$DRY_RUN" = 1 ] && return 0
  [ "$ASSUME_YES" = 1 ] && { say "$prompt — yes (--yes)"; return 0; }
  read -r -p "    $prompt [Y/n] " answer </dev/tty || return 1
  case "$answer" in [nN]*) return 1 ;; *) return 0 ;; esac
}

# Everything that changes the machine goes through this, so --dry-run is one
# check rather than one per step.
run() {
  if [ "$DRY_RUN" = 1 ]; then
    printf '    would run: %s\n' "$*"
    return 0
  fi
  "$@"
}

while [ $# -gt 0 ]; do
  case "$1" in
    --yes|-y) ASSUME_YES=1 ;;
    --dry-run|-n) DRY_RUN=1 ;;
    --uninstall) UNINSTALL=1 ;;
    --help|-h) usage ;;
    *) fail "unknown option: $1"; exit 2 ;;
  esac
  shift
done

# ------------------------------------------------------------- uninstall --

if [ "$UNINSTALL" = 1 ]; then
  step "Uninstall"
  run "$HERE/kotodex/install-entry.sh" --uninstall

  # Nothing here installs this unit — the launcher starts the capture daemon and
  # adopts one already running, so it is optional. It is removed anyway because
  # vn-mine/README.md tells the reader to install it by hand for capture at
  # login, and uninstall has to clean up what the docs told them to create.
  unit="$HOME/.config/systemd/user/kotodex-capture.service"
  if [ -f "$unit" ]; then
    run systemctl --user disable --now kotodex-capture.service
    run rm -f "$unit"
    run systemctl --user daemon-reload
    good "removed the capture unit"
  fi

  # Asked separately and never with --yes: these hold every line ever read and
  # the whole vocabulary ledger, and there is no undo.
  step "Reading history"
  say "$DATA holds your reading history, lookups and vocabulary ledger."
  if [ "$DRY_RUN" = 1 ]; then
    say "would ask before removing it"
  else
    answer=""
    if (exec 2>/dev/null; : </dev/tty); then
      read -r -p "    Delete it? Type DELETE to confirm: " answer </dev/tty || answer=""
    else
      say "no terminal to confirm on — keeping it"
    fi
    if [ "$answer" = "DELETE" ]; then
      rm -rf "$DATA"
      good "removed $DATA"
    else
      good "kept $DATA"
    fi
  fi
  printf '\n'
  exit 0
fi

# -------------------------------------------------------------- platform --

step "This machine"
say "$(_os_release_field PRETTY_NAME 2>/dev/null || echo "unknown system")"
MGR="$(pkg_manager || echo unknown)"
if [ "$MGR" = unknown ]; then
  skip "no package manager recognised — install commands below are generic"
else
  good "package manager: $MGR"
fi

# ----------------------------------------------------------- dependencies --

step "Dependencies"

# Both lists feed one install line at the end of this step. A package named
# where it is missing is a second thing to paste and a second run to notice it,
# so nothing here prints a command of its own.
REQUIRED_MISSING=()
OPTIONAL_MISSING=()
require() {
  local bin="$1" generic="$2"
  if have "$bin"; then good "$bin"; else fail "$bin"; REQUIRED_MISSING+=("$generic"); fi
}
optional() {
  local bin="$1" generic="$2" what="$3"
  if have "$bin"; then good "$bin"; else skip "$bin — $what"; OPTIONAL_MISSING+=("$generic"); fi
}

require curl curl
require jq jq
require unzip unzip
require python3 python
require ffmpeg ffmpeg
require pactl pactl

# A pip PySide6 carries its own Qt and cannot load the system layer-shell
# plugin, so the packaged one is what the layer-shell backend needs. Where the
# distribution has none — Ubuntu 24.04 LTS and Debian 12 carry PySide2 only —
# pip in a venv is the only way to run at all, and those are X11 machines
# anyway.
PYSIDE_PIP=0
if [ "$(pyside6_source 2>/dev/null)" = pip ]; then
  good "PySide6 with Qt WebEngine (pip, in $KOTODEX_VENV)"
elif [ -n "$(pyside6_source 2>/dev/null)" ]; then
  good "PySide6 with Qt WebEngine"
elif distro_packages_pyside6; then
  fail "PySide6 with Qt WebEngine"
  REQUIRED_MISSING+=(pyside6 qt6-webengine)
else
  fail "PySide6 with Qt WebEngine — $(pkg_manager || echo this system) has no package for it"
  say "pip can install it into a venv instead. The overlay then runs on X11,"
  say "which is what this desktop would use anyway."
  if confirm "Install PySide6 with pip into $KOTODEX_VENV? (~200 MB)"; then
    PYSIDE_PIP=1
  else
    skip "no PySide6"
  fi
fi

if [ "$PYSIDE_PIP" = 1 ]; then
  if [ "$DRY_RUN" = 1 ]; then
    say "would create $KOTODEX_VENV and pip install PySide6"
  else
    run python3 -m venv --system-site-packages "$KOTODEX_VENV" \
      && run "$KOTODEX_VENV/bin/pip" install --quiet --upgrade pip \
      && run "$KOTODEX_VENV/bin/pip" install --quiet PySide6
    import_error="$("$KOTODEX_VENV/bin/python" -c "import PySide6.QtWebEngineQuick" 2>&1)"
    if [ -z "$import_error" ]; then
      good "PySide6 installed into $KOTODEX_VENV"
    else
      fail "the pip PySide6 still cannot be imported"
      # Its own message names the missing piece — usually a shared library the
      # wheel expects the system to have, or python3-venv where the venv itself
      # was never made.
      printf '%s\n' "$import_error" | tail -3 | sed 's/^/      /'
      # The wheel carries its own Qt but not the graphics libraries under it.
      # A desktop has these; a minimal or server install does not.
      say "a missing lib*.so comes with the system Qt: $(pkg_install_cmd webengine-runtime)"
      exit 1
    fi
  fi
fi

optional xdotool xdotool "required for anki cards to get a screenshot of the right window, and for positioning of the overlay"
if first="$(for b in spectacle grim gnome-screenshot import; do have "$b" && echo "$b" && break; done)" \
   && [ -n "$first" ]; then
  good "screenshot tool: $first"
else
  skip "no screenshot tool — required for anki cards to get a picture"
  OPTIONAL_MISSING+=(screenshot)
fi

# layer-shell is what puts the overlay over a fullscreen game; the X11 backend
# is the fallback and picks itself, so a missing plugin is not fatal.
if python3 "$HERE/layer-overlay/backend.py" >/dev/null 2>&1; then
  good "overlay backend: $(python3 "$HERE/layer-overlay/backend.py" | cut -f1)"
else
  skip "overlay backend undecided — layer-overlay/backend.py did not answer"
fi

# One line covering both lists, so the optional packages are installed in the
# same paste rather than discovered on a later run.
if [ ${#REQUIRED_MISSING[@]} -gt 0 ]; then
  printf '\n'
  fail "Install these, then run setup.sh again:"
  printf '\n      %s\n\n' "$(pkg_install_cmd "${REQUIRED_MISSING[@]}" "${OPTIONAL_MISSING[@]}")"
  exit 1
elif [ ${#OPTIONAL_MISSING[@]} -gt 0 ]; then
  printf '\n'
  say "Everything needed is here. For the parts marked — above:"
  printf '\n      %s\n' "$(pkg_install_cmd "${OPTIONAL_MISSING[@]}")"
fi

# ---------------------------------------------------------- python packages --

# websockets for the Textractor logger, onnxruntime and numpy for the VAD.
# A venv rather than the system python: these are the interpreter's own
# dependencies, and `--system-site-packages` keeps everything else — including a
# packaged PySide6 — coming from the distribution.
step "Python packages"
REQS="$HERE/vn-mine/requirements.txt"
VENV_PYTHON="$KOTODEX_VENV/bin/python"
venv_imports() { "$VENV_PYTHON" -c "import websockets, onnxruntime, numpy" >/dev/null 2>&1; }

if [ -x "$VENV_PYTHON" ] && venv_imports; then
  good "websockets, onnxruntime, numpy (in $KOTODEX_VENV)"
elif [ "$DRY_RUN" = 1 ]; then
  say "would create $KOTODEX_VENV and install $REQS (~70 MB)"
else
  say "installing into $KOTODEX_VENV (~70 MB)"
  run python3 -m venv --system-site-packages "$KOTODEX_VENV" \
    && run "$VENV_PYTHON" -m pip install --quiet --upgrade pip \
    && run "$VENV_PYTHON" -m pip install --quiet -r "$REQS"
  if venv_imports; then
    good "websockets, onnxruntime, numpy"
  else
    fail "the venv is there but its packages cannot be imported"
    "$VENV_PYTHON" -c "import websockets, onnxruntime, numpy" 2>&1 | tail -3 | sed 's/^/      /'
    # Debian and Ubuntu split venv out of python3, so this is where a machine
    # without it arrives.
    say "if venv itself is missing: $(pkg_install_cmd python3-venv)"
    say "required for capturing the game's text, and for trimming card audio to the spoken line"
  fi
fi

# ---------------------------------------------------------------- binaries --

step "Binaries"
# A checkout builds whenever cargo is here, not only when the binaries are
# missing: one that has moved on leaves binaries that still run and answer with
# stale behaviour, which is far harder to see than a missing one. Cargo is a
# no-op when they are current.
#
# Cargo.toml is what tells the two apart. The tarball ships the binaries and no
# source at all, so asking for cargo alone would run a build with nothing to
# build on any machine that happens to have rustup.
if [ -f "$HERE/Cargo.toml" ] && have cargo; then
  say "building — the first time takes a few minutes"
  # The same three the tarball ships. Building the whole workspace would pull
  # in yt-mine and manga-mine, which have no part in reading a VN.
  run bash -c "cd '$HERE' && cargo build --release -p read-stats \
    -p jp-core --bin jp-dict -p jp-mine-core --bin anki-setup" \
    || { fail "build failed"; exit 1; }
  good "built"
elif [ -x "$HERE/target/release/read-stats" ] && [ -x "$HERE/target/release/jp-dict" ]; then
  good "shipped binaries"
elif [ -f "$HERE/Cargo.toml" ]; then
  fail "no binaries, and no cargo to build this checkout with"
  say "install Rust: https://rustup.rs"
  exit 1
else
  fail "this release is missing its binaries"
  say "re-download the tarball, or build from a git checkout"
  exit 1
fi

# ------------------------------------------------------------------ models --

step "Models"

# size <path> — bytes, or 0.
size_of() { [ -f "$1" ] && wc -c <"$1" || echo 0; }

# A partial download is worse than none: it looks present to every later check.
# Download to a temporary name, verify the size, then move it into place.
fetch() {
  local url="$1" dest="$2" min="$3" label="$4" tmp
  if [ "$DRY_RUN" = 1 ]; then
    printf '    would download: %s\n' "$label"
    return 0
  fi
  tmp="$dest.part"
  say "downloading $label"
  # A progress bar redirected to a file is thousands of lines of hashes.
  local progress=--progress-bar
  [ -t 1 ] || progress=-sS
  if ! curl -fL "$progress" -o "$tmp" "$url"; then
    rm -f "$tmp"
    fail "$label download failed — re-run setup.sh to try again"
    return 1
  fi
  if [ "$(size_of "$tmp")" -lt "$min" ]; then
    rm -f "$tmp"
    fail "$label came back too small to be the real file"
    return 1
  fi
  mv "$tmp" "$dest"
  good "$label"
}

if [ -f "$HERE/system_full.dic" ]; then
  good "SudachiDict (system_full.dic)"
elif [ "$DRY_RUN" = 1 ]; then
  say "would download SudachiDict full (~127 MB, Apache-2.0)"
else
  mkdir -p "$HERE"
  zip="$HERE/sudachi-dict.zip"
  if fetch "$SUDACHI_URL" "$zip" "$SUDACHI_MIN_BYTES" "SudachiDict full (~127 MB, Apache-2.0)"; then
    # The zip nests the dictionary under a dated directory, so -j flattens it
    # rather than the path being guessed at.
    unzip -o -j "$zip" '*/system_full.dic' -d "$HERE" >/dev/null \
      || unzip -o -j "$zip" 'system_full.dic' -d "$HERE" >/dev/null
    rm -f "$zip"
    [ -f "$HERE/system_full.dic" ] && good "SudachiDict unpacked" || fail "no system_full.dic in the zip"
  fi
fi

VAD="$DATA/silero_vad.onnx"
if [ -f "$VAD" ]; then
  good "silero VAD model"
elif [ "$DRY_RUN" = 1 ]; then
  say "would download silero_vad.onnx (2.2 MB, MIT)"
else
  mkdir -p "$DATA"
  fetch "$VAD_URL" "$VAD" "$VAD_MIN_BYTES" "silero VAD model (2.2 MB, MIT)"
fi

# ------------------------------------------------------------ dictionaries --

step "Dictionaries"
mkdir -p "$HERE/dictionaries"

JP_DICT="$HERE/target/release/jp-dict"

# Nothing here is redistributed: each is fetched from whoever publishes it, at
# the version they are publishing today. That is also why the URLs are resolved
# rather than pinned — a stale pin is a dictionary that quietly stops existing.
jitendex_url() {
  curl -sL --max-time 30 https://api.github.com/repos/stephenmk/stephenmk.github.io/releases/latest \
    | jq -r '.assets[] | select(.name == "jitendex-yomitan.zip") | .browser_download_url'
}

# What is already imported, so a dictionary that is present under another
# filename is not downloaded a second time. `source_path` is the cache key, so a
# second copy under a second name is a duplicate row, not a no-op.
IMPORTED=""
[ -x "$JP_DICT" ] && IMPORTED="$("$JP_DICT" list 2>/dev/null)"

# want_dictionary <zip-name> <role-or-title match> <label> — false when it is
# already imported, or already sitting in dictionaries/ waiting to be.
want_dictionary() {
  local zip="$HERE/dictionaries/$1" match="$2" label="$3"
  if [ -f "$zip" ]; then good "$label — already in dictionaries/"; return 1; fi
  if [ -n "$IMPORTED" ] && printf '%s' "$IMPORTED" | grep -qi -- "$match"; then
    good "$label — already imported"
    return 1
  fi
  return 0
}

# Both are free and neither is optional in practice: with no definitions the
# popup is empty, and with no ranks nothing is underlined or ordered. Asking
# only offers the reader a broken install.
if want_dictionary jitendex-yomitan.zip "jitendex" "Jitendex"; then
  url="$(jitendex_url)"
  if [ -n "$url" ] && [ "$url" != null ]; then
    fetch "$url" "$HERE/dictionaries/jitendex-yomitan.zip" 10000000 \
      "Jitendex — Japanese-English (~39 MB, CC BY-SA 4.0)"
  else
    fail "could not resolve the Jitendex release — get it from https://jitendex.org"
  fi
fi

# jiten.moe ranks the media people actually read. HEAD is refused there, so
# fetch has nothing to probe with; the size check on the result is the check.
if want_dictionary jiten-frequency.zip "frequency" "A frequency list"; then
  fetch "https://api.jiten.moe/api/frequency-list/download" \
    "$HERE/dictionaries/jiten-frequency.zip" 3000000 \
    "Jiten frequency list — ranks fiction (~8 MB)"
fi

zips=("$HERE"/dictionaries/*.zip)
if [ -e "${zips[0]}" ]; then
  run "$JP_DICT" sync
elif [ "$DRY_RUN" = 1 ]; then
  say "would import what the downloads above put in dictionaries/"
else
  skip "dictionaries/ is empty — required for any definitions in the popup"
fi

say ""
say "To add more: drop a Yomitan zip in $HERE/dictionaries/ and run setup.sh again."

# -------------------------------------------------------------------- Anki --

step "Anki"
ANKI_SETUP="$HERE/target/release/anki-setup"
if [ "$DRY_RUN" = 1 ]; then
  say "would run anki-setup check"
else
  report="$("$ANKI_SETUP" check 2>&1)"; ok=$?
  printf '%s\n' "$report" | sed 's/^/    /'
  if [ "$ok" != 0 ]; then
    if printf '%s' "$report" | grep -q "install-lapis" \
       && confirm "Import the Lapis note type into Anki?"; then
      "$ANKI_SETUP" install-lapis 2>&1 | sed 's/^/    /'
    else
      say "no mining until this is fixed; everything else works"
    fi
  fi
fi

# ------------------------------------------------------------------ extras --

step "Extras"

# The key lives in .env because that is the file every crate already loads
# (dotenvy, from the install directory). 600 because it is a credential.
set_env_var() {
  local key="$1" value="$2"
  run touch "$ENV_FILE"
  run chmod 600 "$ENV_FILE"
  [ "$DRY_RUN" = 1 ] && { printf '    would set %s in %s\n' "$key" "$ENV_FILE"; return 0; }
  grep -v "^$key=" "$ENV_FILE" >"$ENV_FILE.new" 2>/dev/null || true
  printf '%s=%s\n' "$key" "$value" >>"$ENV_FILE.new"
  mv "$ENV_FILE.new" "$ENV_FILE"
  chmod 600 "$ENV_FILE"
}

if grep -q '^KOTODEX_ANTHROPIC_API_KEY=.' "$ENV_FILE" 2>/dev/null; then
  good "Anthropic API key set — AI explanations are on"
elif [ "$DRY_RUN" = 1 ]; then
  say "would offer to store an Anthropic API key"
elif ! (exec 2>/dev/null; : </dev/tty); then
  skip "no terminal to type a key into — set KOTODEX_ANTHROPIC_API_KEY in $ENV_FILE"
elif confirm "Add an Anthropic API key? It enables AI generated explanation of lines, and word definitions."; then
  key=""
  read -r -s -p "    key (not echoed, blank to skip): " key </dev/tty || true
  printf '\n'
  if [ -n "$key" ]; then
    set_env_var KOTODEX_ANTHROPIC_API_KEY "$key"
    good "stored in $ENV_FILE (600)"
  else
    skip "no key"
  fi
else
  skip "no key"
fi

# Nothing here installs whisper, so this only ever reports. Probed rather than
# asserted: it said "no whisper" to a reader with the service running, while
# doctor two steps later said "reachable" off the same endpoint.
WHISPER_URL="${KOTODEX_WHISPER_URL:-http://localhost:8100}"
if curl -fsS --max-time 2 "$WHISPER_URL/health" >/dev/null 2>&1; then
  good "whisper answering on $WHISPER_URL"
else
  skip "no whisper — required for trimming card audio to the mined sentence"
  say "set it up by hand: whisper-service/README.md"
fi

# ----------------------------------------------------------- application --

step "Application entry"
run "$HERE/kotodex/install-entry.sh"

# ------------------------------------------------------------------ doctor --

step "Anything still missing"
if [ "$DRY_RUN" = 1 ]; then
  say "would run scripts/kotodex-doctor.sh --only-problems"
  doctor=0
else
  # Only the problems: every step above has just reported itself, and repeating
  # the whole table buries the two rows that need something. The full table is
  # scripts/kotodex-doctor.sh.
  "$HERE/scripts/kotodex-doctor.sh" --only-problems
  doctor=$?
fi

printf '\n'
if [ "$DRY_RUN" = 1 ]; then
  :
elif [ "$doctor" = 0 ]; then
  printf '%sReady.%s\n' "$green" "$off"
else
  # The doctor has already named what is wrong, and it is not always something
  # to install — an unstarted service is the commonest ✗ on a fresh install.
  printf '%sNot ready yet.%s Do what the ✗ rows above say, then run ./setup.sh again.\n' "$yellow" "$off"
fi

# Restated because they scrolled past: each of these was mentioned once, in the
# middle of a step that was doing something else at the time.
printf '\n'
printf '  %-20s %s\n' "start it" "kotodex — or from the application menu"
printf '  %-20s %s\n' "check what works" "scripts/kotodex-doctor.sh"
printf '  %-20s %s\n' "add a dictionary" "drop a Yomitan zip in dictionaries/, then ./setup.sh again"
printf '  %-20s %s\n' "uninstall" "./setup.sh --uninstall"
printf '  %-20s %s\n' "guide" "TODO"
printf '\n'
exit "$doctor"
