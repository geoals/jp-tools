#!/usr/bin/env bash
# What works here, what does not, and the one command that fixes each.
#
#   scripts/kotodex-doctor.sh [--url http://localhost:3200]
#
# Exit 0 when the core works: a tokenizer dictionary, at least one definition
# dictionary, and a line source. Everything else is reported and forgiven — the
# product degrades rather than fails, and this is the page that says how far.
#
# The rows come from read-stats' capability probe, which is the same table
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

URL="http://localhost:${READ_STATS_PORT:-3200}"
[ "${1:-}" = "--url" ] && URL="$2"

bold=$'\033[1m'; red=$'\033[31m'; yellow=$'\033[33m'; green=$'\033[32m'; off=$'\033[0m'
[ -t 1 ] || { bold=""; red=""; yellow=""; green=""; off=""; }

core_broken=0

section() { printf '\n%s%s%s\n' "$bold" "$1" "$off"; }

# row <ok> <name> <detail> <fix> <critical>
row() {
  local ok="$1" name="$2" detail="$3" fix="$4" critical="${5:-}"
  if [ "$ok" = true ]; then
    printf '  %s✓%s %-18s %s\n' "$green" "$off" "$name" "$detail"
    return
  fi
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

printf '%sKotodex%s — %s\n' "$bold" "$off" "$(_os_release_field PRETTY_NAME 2>/dev/null || echo "unknown system")"

section "Core"
binary_row curl curl critical
binary_row jq jq critical
if [ -f "$REPO/system_full.dic" ] || [ -n "${KOTODEX_SUDACHI_DICT_PATH:-}" ]; then
  row true "SudachiDict" "present" ""
else
  row false "SudachiDict" "missing" "run setup.sh — nothing tokenizes without it" critical
fi
if [ -n "$CAPS" ]; then
  row true "read-stats" "answering on $URL" ""
else
  row false "read-stats" "not answering on $URL" \
    "start it: scripts/start-all.sh read-stats — the rows below need it" critical
fi

section "Capture"
binary_row pactl pactl
binary_row ffmpeg ffmpeg
cap capture_running "ring buffer"
cap lines_source "line source"
cap vad_model "VAD model"
cap whisper "whisper"

section "Dictionaries"
cap dict_master "master"
cap dict_definitions "definitions" critical
cap dict_frequency "frequency"
cap dict_pitch "pitch"
cap vocabulary_ledger "ledger"

section "Anki"
cap anki "AnkiConnect"
cap anki_note_type "note type"
cap screenshot_tool "screenshot tool"

section "Overlay"
py="$(kotodex_python)"
if [ "$py" != python3 ]; then
  row true "PySide6" "pip, in $KOTODEX_VENV" ""
elif python3 -c "import PySide6.QtWebEngineQuick" >/dev/null 2>&1; then
  row true "PySide6" "installed" ""
elif distro_packages_pyside6; then
  row false "PySide6" "not installed" "$(pkg_install_cmd pyside6 qt6-webengine)" critical
else
  row false "PySide6" "not installed, and $(pkg_manager || echo this system) has no package for it" \
    "re-run setup.sh — it can install one with pip into a venv" critical
fi
cap overlay_backend "backend"
cap xdotool "window tracking"

section "Extras"
cap explain "explain"
printf '  %s—%s %-18s %s\n' "$yellow" "$off" "whisper setup" \
  "not configured automatically in this release"
printf '      it narrows a card'"'"'s clip to the mined sentence; see whisper-service/README.md\n'

if [ -z "$CAPS" ]; then
  printf '\n%sMost rows need read-stats running.%s Start it and run this again.\n' "$yellow" "$off"
fi

printf '\n'
if [ "$core_broken" = 0 ]; then
  printf '%sThe core works.%s Anything marked — is optional and says what it would add.\n' "$green" "$off"
  exit 0
fi
printf '%sSomething the core needs is missing.%s The ✗ rows above say what.\n' "$red" "$off"
exit 1
