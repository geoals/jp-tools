# Run a layer_overlay.run() script detached from the shell that started it.
# Sourced, not executed:
#
#   OVERLAY_NAME=my-overlay
#   OVERLAY_SCRIPT="$HERE/my-overlay.py"
#   source /path/to/layer-overlay/runner.sh
#   layer_overlay_main "$@"
#
# Gives the caller `start` / `stop` / `restart` / `status`, and passes anything
# after the command through to the script. `-h`/`--help` is the caller's, so it
# can document its own flags — handle it before calling this.
#
#   OVERLAY_RUN_DIR   pid and log live here  (default $XDG_RUNTIME_DIR/$OVERLAY_NAME)
#   OVERLAY_LOG       where output goes      (default $OVERLAY_RUN_DIR/overlay.log)
#   WAYLAND_DISPLAY   compositor socket      (default: the only one running)
#
# Started over ssh the script has no session of its own to inherit, so this
# fills in what a Wayland client needs: the runtime dir, the compositor socket,
# and the Qt platform plugin. `setsid` plus closed stdio is what keeps it alive
# when the ssh session ends — a layer surface has no terminal to belong to.

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
# The Qt platform plugin is the backend's to set — layer-shell needs wayland
# and the X11 backend needs xcb, and only layer_overlay.py knows which it
# picked. Anything already exported here still wins.

OVERLAY_RUN_DIR="${OVERLAY_RUN_DIR:-$XDG_RUNTIME_DIR/$OVERLAY_NAME}"
OVERLAY_LOG="${OVERLAY_LOG:-$OVERLAY_RUN_DIR/overlay.log}"
_OVERLAY_PID_FILE="$OVERLAY_RUN_DIR/overlay.pid"

# The pid of the running overlay, or empty. The pid file alone is not enough —
# it outlives a crash, and the number is reused — so the process behind it has
# to still be this script. `pgrep -f` is the fallback, so that an instance
# started by hand is found and replaced rather than run twice.
layer_overlay_pid() {
  local pid
  if [[ -r "$_OVERLAY_PID_FILE" ]] && read -r pid <"$_OVERLAY_PID_FILE" && [[ -n "$pid" ]]; then
    if grep -qz -- "$OVERLAY_SCRIPT" "/proc/$pid/cmdline" 2>/dev/null; then
      echo "$pid"
      return
    fi
  fi
  # `|| true`: no match is the ordinary answer here, not a failure, and under
  # `pipefail` pgrep's 1 would otherwise end the script.
  pgrep -f -- "python3? .*$(basename "$OVERLAY_SCRIPT")" | head -1 || true
}

layer_overlay_stop() {
  local pid tries=0
  pid="$(layer_overlay_pid)"
  [[ -z "$pid" ]] && return 0
  echo "stopping $OVERLAY_NAME (pid $pid)"
  kill "$pid" 2>/dev/null || true
  while kill -0 "$pid" 2>/dev/null; do
    ((tries += 1))
    if ((tries > 20)); then
      echo "$OVERLAY_NAME did not exit after 10s — sending SIGKILL" >&2
      kill -9 "$pid" 2>/dev/null || true
      break
    fi
    sleep 0.5
  done
  rm -f "$_OVERLAY_PID_FILE"
}

# One compositor is the normal case, so naming its socket is only necessary
# when there are several. Guessing between them would put the overlay on a
# screen nobody is looking at.
_layer_overlay_wayland() {
  [[ -n "${WAYLAND_DISPLAY-}" ]] && return 0
  local sockets
  mapfile -t sockets < <(cd "$XDG_RUNTIME_DIR" && ls -1 wayland-[0-9]* 2>/dev/null | grep -v '\.lock$')
  # No socket at all is not an error: an X11 session is where the X11 backend
  # is the whole point, and saying which backend applies is its job, not this
  # one's.
  if ((${#sockets[@]} == 0)); then
    return 0
  fi
  if ((${#sockets[@]} > 1)); then
    echo "several wayland sockets (${sockets[*]}) — set WAYLAND_DISPLAY" >&2
    return 1
  fi
  export WAYLAND_DISPLAY="${sockets[0]}"
}

layer_overlay_main() {
  local command="start"
  case "${1-}" in
    start | stop | restart | status)
      command="$1"
      shift
      ;;
  esac
  [[ "$command" == "restart" ]] && command="start"

  case "$command" in
    stop)
      layer_overlay_stop
      return 0
      ;;
    status)
      local pid
      pid="$(layer_overlay_pid)"
      if [[ -z "$pid" ]]; then
        echo "$OVERLAY_NAME: not running"
        return 1
      fi
      echo "$OVERLAY_NAME: running (pid $pid)"
      tr '\0' ' ' <"/proc/$pid/cmdline"
      echo
      return 0
      ;;
  esac

  mkdir -p "$OVERLAY_RUN_DIR"
  layer_overlay_stop
  _layer_overlay_wayland || return 1

  # setsid so it leaves the ssh session's process group, and every descriptor
  # redirected so nothing of it is left pointing at a terminal that is about to
  # close. Without both, logging out takes the overlay with it.
  setsid python3 "$OVERLAY_SCRIPT" "$@" </dev/null >"$OVERLAY_LOG" 2>&1 &
  local pid=$!
  echo "$pid" >"$_OVERLAY_PID_FILE"

  # It fails loudly and early — no Wayland socket, no layer-shell, a QML error —
  # so a moment's wait turns "started" into something worth trusting.
  sleep 1
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "$OVERLAY_NAME exited immediately — $OVERLAY_LOG says:" >&2
    tail -n 20 "$OVERLAY_LOG" >&2
    rm -f "$_OVERLAY_PID_FILE"
    return 1
  fi
  echo "$OVERLAY_NAME: running (pid $pid), log $OVERLAY_LOG"
  grep -m1 '^backend:' "$OVERLAY_LOG" 2>/dev/null || true
}
