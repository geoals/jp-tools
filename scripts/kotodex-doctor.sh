#!/usr/bin/env bash
# What works here, what does not, and the one command that fixes each.
#
#   scripts/kotodex-doctor.sh [--url http://localhost:3200] [--only-problems]
#                             [--core | --full]
#
# `--only-problems` prints just the rows that need something and the sections
# holding them, which is what setup.sh ends with: a reader who has just watched
# every step succeed does not need the same list again.
#
# Exit 0 when the core works: curl, jq, a tokenizer dictionary, a kotodex-server
# answering, at least one definition dictionary, and a PySide6 to draw the
# overlay with. Everything else is reported and forgiven — the product degrades
# rather than fails, and this is the page that says how far. A row is `critical`
# here exactly when `docs/degradation.md` calls it required.
#
# The rows come from kotodex-server's capability probe, which is the same table
# `docs/degradation.md` describes and the reading surfaces draw from. One
# implementation: the installer's closing summary is this script.
set -uo pipefail

# Bash's own expansion rather than dirname/readlink: this is the one script that
# has to survive a system missing the things it is about to report as missing.
REPO="${BASH_SOURCE[0]%/*}/.."
if [ -r "$REPO/scripts/lib/platform.sh" ]; then
  source "$REPO/scripts/lib/platform.sh"
else
  pkg_install_cmd() { echo "install: $*"; }
  _os_release_field() { return 1; }
fi

URL="http://localhost:${SERVER_PORT:-3200}"
ONLY_PROBLEMS=0
# Run from setup.sh, which deliberately leaves Kotodex stopped. A row whose
# only remedy is starting it is then the expected state of every fresh install,
# and listing it under "anything still missing" made a finished install read as
# a broken one.
INSTALLING=0

# Which tier setup.sh installed. Absent means full, which is what every install
# made before the tiers existed is — reporting a missing overlay as a fault on
# one of those is right.
TIER_FILE="${KOTODEX_DATA:-$HOME/.local/share/kotodex}/install-tier"
TIER="$( [ -r "$TIER_FILE" ] && cat "$TIER_FILE" || echo full )"

while [ $# -gt 0 ]; do
  case "$1" in
    --url) URL="$2"; shift ;;
    --only-problems) ONLY_PROBLEMS=1 ;;
    --installing) INSTALLING=1 ;;
    --core) TIER=core ;;
    --full) TIER=full ;;
  esac
  shift
done

# A core install reads nothing on this machine: no game window to screenshot,
# no speakers to record, no overlay to draw. Checking for those would report
# every one of them as broken on a machine doing exactly what it was set up to
# do.
reading_here() { [ "$TIER" = full ]; }

bold=$'\033[1m'; red=$'\033[31m'; yellow=$'\033[33m'; green=$'\033[32m'; off=$'\033[0m'
[ -t 1 ] || { bold=""; red=""; yellow=""; green=""; off=""; }

core_broken=0

# Held rather than printed, so a section whose every row was suppressed does not
# leave a bare heading behind.
PENDING_SECTION=""
# The banner is held for the same reason: --only-problems on a healthy machine
# printed nothing but a title, which read as a report that had lost its rows.
PENDING_TITLE=""
flush_title() {
  [ -n "$PENDING_TITLE" ] || return 0
  printf '%s' "$PENDING_TITLE"
  PENDING_TITLE=""
}
section() {
  if [ "$ONLY_PROBLEMS" = 1 ]; then PENDING_SECTION="$1"; return; fi
  printf '\n%s%s%s\n' "$bold" "$1" "$off"
}
flush_section() {
  [ -n "$PENDING_SECTION" ] || return 0
  flush_title
  printf '\n%s%s%s\n' "$bold" "$PENDING_SECTION" "$off"
  PENDING_SECTION=""
}

# row <ok> <name> <detail> <fix> <critical>
row() {
  local ok="$1" name="$2" detail="$3" fix="$4" critical="${5:-}"
  if [ "$ok" = true ]; then
    [ "$ONLY_PROBLEMS" = 1 ] && return
    printf '  %s✓%s %-18s %s\n' "$green" "$off" "$name" "$detail"
    return
  fi
  case "$INSTALLING$fix" in 1"start Kotodex"*) return ;; esac
  flush_section
  if [ -n "$critical" ]; then
    printf '  %s✗%s %-18s %s\n' "$red" "$off" "$name" "$detail"
    core_broken=1
  else
    printf '  %s—%s %-18s %s\n' "$yellow" "$off" "$name" "$detail"
  fi
  [ -n "$fix" ] && printf '      %s\n' "$fix"
}

have() { command -v "$1" >/dev/null 2>&1; }

# One row per command that must be on PATH, with a paste-able install line.
binary_row() {
  local bin="$1" generic="$2" critical="${3:-}"
  if have "$bin"; then
    row true "$bin" "installed" ""
  else
    row false "$bin" "not installed" "$(pkg_install_cmd "$generic")" "$critical"
  fi
}

STATE=""
if have curl; then
  STATE="$(curl -s --max-time 3 "$URL/api/reader/state" 2>/dev/null)"
fi
CAPS=""
if [ -n "$STATE" ] && have jq; then
  CAPS="$(printf '%s' "$STATE" | jq -c '.capabilities // empty' 2>/dev/null)"
fi

# cap <key> <label> <critical>
cap() {
  local key="$1" label="$2" critical="${3:-}"
  [ -n "$CAPS" ] || return 1
  local ok detail fix
  # `// empty` is wrong here: jq treats `false` as empty, which silently drops
  # every row that is off — the ones worth printing.
  ok="$(printf '%s' "$CAPS" | jq -r --arg k "$key" 'if has($k) then .[$k].ok else empty end')"
  [ -n "$ok" ] || return 1
  detail="$(printf '%s' "$CAPS" | jq -r --arg k "$key" '.[$k].detail // ""')"
  fix="$(printf '%s' "$CAPS" | jq -r --arg k "$key" '.[$k].fix // ""')"
  row "$ok" "$label" "$detail" "$fix" "$critical"
}

TITLE="$(printf '%sKotodex%s — %s\n' "$bold" "$off" \
  "$(_os_release_field PRETTY_NAME 2>/dev/null || echo "unknown system")")"
if [ "$ONLY_PROBLEMS" = 1 ]; then
  PENDING_TITLE="$TITLE
"
else
  printf '%s\n' "$TITLE"
fi

section "Core"
binary_row curl curl critical
binary_row jq jq critical
if [ -f "$REPO/system_full.dic" ] || [ -n "${KOTODEX_SUDACHI_DICT_PATH:-}" ]; then
  row true "SudachiDict" "present" ""
else
  row false "SudachiDict" "missing" "run setup.sh — required for reading any Japanese text" critical
fi
if [ -n "$CAPS" ]; then
  row true "kotodex-server" "answering on $URL" ""
else
  if reading_here; then
    start="start Kotodex from the application menu"
  else
    start="start it with target/release/kotodex-server"
  fi
  row false "kotodex-server" "not answering on $URL" \
    "$start — the rows below need it" critical
fi

if reading_here; then
section "Capture"
binary_row pactl pactl
binary_row ffmpeg ffmpeg
if have ffmpeg; then
  if ffmpeg_records_pulse; then
    row true "ffmpeg audio in" "pulse" ""
  else
    row false "ffmpeg audio in" "$(command -v ffmpeg) has no pulse input" \
      "put an ffmpeg with PulseAudio support first on PATH (on Fedora, the rpmfusion build) — required for anki cards to get the voice clip of the line being mined"
  fi
fi
cap capture_running "ring buffer"
cap lines_source "line source"
cap vad_model "VAD model"
cap whisper "whisper"
fi  # reading_here

section "Dictionaries"
cap dict_master "master"
cap dict_definitions "definitions" critical
cap dict_frequency "frequency"
cap dict_pitch "pitch"
cap vocabulary_ledger "ledger"

section "Anki"
cap anki "AnkiConnect"
cap anki_note_type "note type"
reading_here && cap screenshot_tool "screenshot tool"

if reading_here; then
section "Overlay"
src="$(pyside6_source 2>/dev/null || true)"
if [ "$src" = pip ]; then
  row true "PySide6" "pip, in $KOTODEX_VENV" ""
elif [ -n "$src" ]; then
  row true "PySide6" "installed" ""
elif distro_packages_pyside6; then
  row false "PySide6" "not installed" "$(pkg_install_cmd pyside6 qt6-webengine)" critical
else
  row false "PySide6" "not installed, and $(pkg_manager || echo this system) has no package for it" \
    "re-run setup.sh — it can install one with pip into a venv" critical
fi
cap overlay_backend "backend"
cap xdotool "window tracking"
fi  # reading_here

section "Extras"
cap explain "explain"

if [ -z "$CAPS" ]; then
  flush_title
  printf '\n%sMost rows need Kotodex running.%s Start it, then run this again.\n' "$yellow" "$off"
fi

# Nothing to say and nothing said: a clean --only-problems run ends silently, so
# the step that runs it does not print a heading over an empty report.
if [ "$core_broken" = 0 ]; then
  [ "$ONLY_PROBLEMS" = 1 ] && exit 0
  printf '\n%sThe core works.%s Anything marked — is optional and says what it would add.\n' "$green" "$off"
  exit 0
fi
printf '\n%sSomething the core needs is missing.%s The ✗ rows above say what.\n' "$red" "$off"
exit 1
