#!/usr/bin/env bash
#
# vn-overlay.sh — run the VN reading overlay detached from the shell that
# started it.
#
#   vn-overlay.sh [start] [--mobile ...]  start it, replacing a running one
#   vn-overlay.sh stop                    stop it
#   vn-overlay.sh restart [--mobile ...]  same as start
#   vn-overlay.sh ensure [--mobile ...]   start it only if none is running
#   vn-overlay.sh status                  is it up, and with what
#
# Everything after the command goes to vn-overlay.py, so `--mobile` and any
# `VAR=value` in the environment work exactly as they do there.
#
#   VN_OVERLAY_FONT   font for the line   (default DNP Shuei Mincho Pr6)
#   VN_OVERLAY_LOG    where output goes   (default $XDG_RUNTIME_DIR/vn-overlay/)
#   WAYLAND_DISPLAY   compositor socket   (default: the only one running)
#
# The rest of vn-overlay.py's own variables are passed straight through.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ "${1-}" == "-h" || "${1-}" == "--help" ]]; then
  awk 'NR>1 && /^#/ { sub(/^# ?/, ""); print; next } NR>1 { exit }' "${BASH_SOURCE[0]}"
  exit 0
fi

export VN_OVERLAY_FONT="${VN_OVERLAY_FONT:-DNP Shuei Mincho Pr6}"

# Where the distribution packages no PySide6, setup.sh leaves one in a venv.
source "$HERE/../../scripts/lib/platform.sh"
OVERLAY_PYTHON="$(kotodex_python)"

OVERLAY_NAME="vn-overlay"
OVERLAY_SCRIPT="$HERE/vn-overlay.py"
OVERLAY_LOG="${VN_OVERLAY_LOG:-}"
source "$HERE/../../layer-overlay/runner.sh"

layer_overlay_main "$@"
