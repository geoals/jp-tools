#!/usr/bin/env bash
#
# vn.sh — start/stop/restart just the VN reading stack.
#
# The VN reading stack is a subset of the full jp-tools stack:
#
#   vn-buffer       systemd --user unit: audio ring buffer + line logger.
#                   This is what captures reading time/chars and voiceline
#                   audio. Runs on its own; managed with systemctl (see below).
#   read-stats      :3200  the dashboard + the #read reader/mine page.
#   whisper-service :8100  OPTIONAL. Only sharpens vn-capture.sh's clip trim
#                   (narrowing to the mined sentence). Mining works without it
#                   — the clip is attached VAD-trimmed — and #read shows a
#                   "✂ off" hint when it's down.
#
# yt-mine and manga-ocr-service are unrelated to VN reading and are not touched.
#
#   scripts/vn.sh                 start read-stats + whisper-service
#   scripts/vn.sh status          show them plus the vn-buffer unit
#   scripts/vn.sh stop            stop read-stats + whisper-service
#   scripts/vn.sh restart         restart both, no prompts
#   scripts/vn.sh no-whisper      start read-stats only (skip whisper entirely)
#   scripts/vn.sh restart stats   act on just read-stats (naming a service
#                                 narrows to it, same as start-all.sh)
#
# All other flags (--release, --cpu, -k/--keep) pass straight through to
# start-all.sh. vn-buffer is a login-session daemon, so start/stop it with
# `systemctl --user start|stop|restart vn-buffer`; this script only reports it.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
START_ALL="$SCRIPT_DIR/start-all.sh"

# The VN subset, used when no specific service is named. Dropping
# whisper-service (no-whisper) still leaves a fully working reader; only the
# sentence-level trim goes away.
DEFAULT_SERVICES=(whisper-service read-stats)

# Split args into the command word (defaults to start), any explicitly named VN
# services, and everything else (flags like --release) which passes through.
# Naming a service narrows the action to it rather than appending to the
# default set, so `vn.sh restart stats` doesn't also cycle whisper's container.
COMMAND="start"
named=()
rest=()
for arg in "$@"; do
  case "$arg" in
    -h|--help)                 awk 'NR>1 && /^#/ { sub(/^# ?/, ""); print; next } NR>1 { exit }' \
                                 "${BASH_SOURCE[0]}"; exit 0 ;;
    no-whisper)                DEFAULT_SERVICES=(read-stats) ;;
    start|stop|status|restart) COMMAND="$arg" ;;
    read-stats|stats|whisper-service|whisper) named+=("$arg") ;;
    *)                         rest+=("$arg") ;;
  esac
done

if (( ${#named[@]} > 0 )); then
  SERVICES=("${named[@]}")
else
  SERVICES=("${DEFAULT_SERVICES[@]}")
fi

vn_buffer_status() {
  local state
  state="$(systemctl --user is-active vn-buffer 2>/dev/null || true)"
  printf '%-20s %-7s %-10s ' "vn-buffer" "-" "-"
  if [[ "$state" == "active" ]]; then
    printf '\033[1;32m%s\033[0m (systemd --user)\n' "running"
  else
    printf '\033[1;31m%s\033[0m %s\n' "stopped" "(systemctl --user start vn-buffer)"
  fi
}

"$START_ALL" "$COMMAND" "${rest[@]}" "${SERVICES[@]}"

# start-all.sh doesn't know about the systemd unit; append it to status output.
if [[ "$COMMAND" == "status" ]]; then
  vn_buffer_status
fi
