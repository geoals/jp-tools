#!/bin/bash
# VN mine ring-buffer daemon.
# Keeps the last ~300s of desktop audio as 5s WAV segments in a tmpfs ring,
# and logs every Japanese line Textractor hooks (read from its WebSocket
# server) with a timestamp. vn-capture.sh reads both to cut out the last
# voiceline. Run via systemd user unit: systemctl --user start vn-buffer

RUNDIR="${VN_RUNDIR:-${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/vn-mine}"
SEGDIR="$RUNDIR/seg"
LINES_LOG="$RUNDIR/lines.log"
SEG_TIME=5
SEG_WRAP=60 # 60 x 5s = 300s ring

WS_PYTHON="$HOME/.local/share/vn-mine/venv/bin/python"
WS_LOGGER="$(dirname "$(readlink -f "$0")")/vn-ws-logger.py"

case "$1" in
run)
  for cmd in pactl ffmpeg; do
    if ! command -v "$cmd" &>/dev/null; then
      echo "Error: $cmd is not installed"
      exit 1
    fi
  done
  if ! [ -x "$WS_PYTHON" ] || ! "$WS_PYTHON" -c 'import websockets' 2>/dev/null; then
    echo "Error: $WS_PYTHON with the 'websockets' package is required"
    echo "  $WS_PYTHON -m pip install websockets"
    exit 1
  fi

  mkdir -p "$SEGDIR"
  rm -f "$SEGDIR"/seg*.wav
  : >"$LINES_LOG"

  SINK="${VN_SINK:-$(pactl get-default-sink)}"
  if [ -z "$SINK" ]; then
    echo "Error: could not determine default audio sink"
    exit 1
  fi
  echo "Recording ring buffer from ${SINK}.monitor (${SEG_TIME}s x ${SEG_WRAP} segments)"
  echo "Logging hooked lines from ${VN_WS_URL:-ws://localhost:6677}"

  "$WS_PYTHON" "$WS_LOGGER" &
  WS_PID=$!
  trap 'kill $WS_PID $FF_PID 2>/dev/null' EXIT TERM INT

  # -fflags +bitexact keeps the WAV header at exactly 44 bytes,
  # which vn-capture.sh relies on when concatenating segments.
  ffmpeg -nostdin -hide_banner -loglevel warning \
    -f pulse -i "${SINK}.monitor" \
    -map 0:a -ac 2 -ar 48000 -c:a pcm_s16le \
    -f segment -segment_time "$SEG_TIME" -segment_wrap "$SEG_WRAP" \
    -segment_format wav -fflags +bitexact \
    "$SEGDIR/seg%02d.wav" &
  FF_PID=$!

  # if either child dies, exit so systemd restarts the pair
  wait -n
  exit 1
  ;;

*)
  echo "usage: $0 run"
  exit 2
  ;;
esac
