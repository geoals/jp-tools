#!/usr/bin/env bash
#
# vn-overlay.sh — run the overlay detached from the shell that started it.
#
#   vn-overlay.sh [start] [--mobile ...]  start it, replacing a running one
#   vn-overlay.sh stop                    stop it
#   vn-overlay.sh restart [--mobile ...]  same as start
#   vn-overlay.sh status                  is it up, and with what
#
# Everything after the command goes to vn-overlay.py, so `--mobile` and any
# `VAR=value` in the environment work exactly as they do there.
#
# Started over ssh it has no session of its own to inherit, so this fills in
# what a Wayland client needs: the runtime dir, the compositor socket, and the
# Qt platform plugin. `setsid` plus closed stdio is what keeps it alive when
# the ssh session ends — a layer surface has no terminal to belong to.
#
#   VN_OVERLAY_FONT   font for the line   (default DNP Shuei Mincho Pr6)
#   WAYLAND_DISPLAY   compositor socket   (default: the only one running)
#   VN_OVERLAY_LOG    where output goes   (default $XDG_RUNTIME_DIR/vn-mine/)
#
# The rest of vn-overlay.py's own variables are passed straight through.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/vn-overlay.py"

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
export QT_QPA_PLATFORM="${QT_QPA_PLATFORM:-wayland}"
export VN_OVERLAY_FONT="${VN_OVERLAY_FONT:-DNP Shuei Mincho Pr6}"

RUN_DIR="$XDG_RUNTIME_DIR/vn-mine"
PID_FILE="$RUN_DIR/overlay.pid"
LOG="${VN_OVERLAY_LOG:-$RUN_DIR/overlay.log}"

COMMAND="start"
case "${1-}" in
  start | stop | restart | status)
    COMMAND="$1"
    shift
    ;;
  -h | --help)
    awk 'NR>1 && /^#/ { sub(/^# ?/, ""); print; next } NR>1 { exit }' "${BASH_SOURCE[0]}"
    exit 0
    ;;
esac
[[ "$COMMAND" == "restart" ]] && COMMAND="start"

# The pid of the running overlay, or empty. The pid file alone is not enough —
# it outlives a crash, and the number is reused — so the process behind it has
# to still be this script. `pgrep -f` is the fallback, so that an instance
# started by hand is found and replaced rather than run twice.
overlay_pid() {
  local pid
  if [[ -r "$PID_FILE" ]] && read -r pid <"$PID_FILE" && [[ -n "$pid" ]]; then
    if grep -qz -- "$SCRIPT" "/proc/$pid/cmdline" 2>/dev/null; then
      echo "$pid"
      return
    fi
  fi
  # `|| true`: no match is the ordinary answer here, not a failure, and under
  # `pipefail` pgrep's 1 would otherwise end the script.
  pgrep -f -- 'python3? .*vn-overlay\.py' | head -1 || true
}

stop_overlay() {
  local pid tries=0
  pid="$(overlay_pid)"
  [[ -z "$pid" ]] && return 0
  echo "stopping overlay (pid $pid)"
  kill "$pid" 2>/dev/null || true
  while kill -0 "$pid" 2>/dev/null; do
    ((tries += 1))
    if ((tries > 20)); then
      echo "overlay did not exit after 10s — sending SIGKILL" >&2
      kill -9 "$pid" 2>/dev/null || true
      break
    fi
    sleep 0.5
  done
  rm -f "$PID_FILE"
}

case "$COMMAND" in
  stop)
    stop_overlay
    exit 0
    ;;
  status)
    pid="$(overlay_pid)"
    if [[ -z "$pid" ]]; then
      echo "overlay: not running"
      exit 1
    fi
    echo "overlay: running (pid $pid)"
    tr '\0' ' ' <"/proc/$pid/cmdline"
    echo
    exit 0
    ;;
esac

mkdir -p "$RUN_DIR"
stop_overlay

# One compositor is the normal case, so naming its socket is only necessary
# when there are several. Guessing between them would put the overlay on a
# screen nobody is looking at.
if [[ -z "${WAYLAND_DISPLAY-}" ]]; then
  mapfile -t sockets < <(cd "$XDG_RUNTIME_DIR" && ls -1 wayland-[0-9]* 2>/dev/null | grep -v '\.lock$')
  if ((${#sockets[@]} == 0)); then
    echo "no wayland socket in $XDG_RUNTIME_DIR — is the desktop session running?" >&2
    exit 1
  fi
  if ((${#sockets[@]} > 1)); then
    echo "several wayland sockets (${sockets[*]}) — set WAYLAND_DISPLAY" >&2
    exit 1
  fi
  export WAYLAND_DISPLAY="${sockets[0]}"
fi

# setsid so it leaves the ssh session's process group, and every descriptor
# redirected so nothing of it is left pointing at a terminal that is about to
# close. Without both, logging out takes the overlay with it.
setsid python3 "$SCRIPT" "$@" </dev/null >"$LOG" 2>&1 &
pid=$!
echo "$pid" >"$PID_FILE"

# It fails loudly and early — no Wayland socket, no layer-shell, a QML error —
# so a moment's wait turns "started" into something worth trusting.
sleep 1
if ! kill -0 "$pid" 2>/dev/null; then
  echo "overlay exited immediately — $LOG says:" >&2
  tail -n 20 "$LOG" >&2
  rm -f "$PID_FILE"
  exit 1
fi
echo "overlay: running (pid $pid) on $WAYLAND_DISPLAY, log $LOG"
