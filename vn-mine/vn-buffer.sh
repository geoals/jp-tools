#!/bin/bash
# VN mine ring-buffer daemon.
# Keeps the last ~300s of desktop audio as 5s WAV segments in a tmpfs ring,
# and logs every Japanese clipboard change (Textractor hooked lines) with a
# timestamp. vn-capture.sh reads both to cut out the last voiceline.
# Run via systemd user unit: systemctl --user start vn-buffer

RUNDIR="${VN_RUNDIR:-${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/vn-mine}"
SEGDIR="$RUNDIR/seg"
LINES_LOG="$RUNDIR/lines.log"
START_MARK="$RUNDIR/.start"
SEG_TIME=5
SEG_WRAP=60 # 60 x 5s = 300s ring

case "$1" in
_log)
  # invoked by wl-paste --watch with clipboard content on stdin
  text=$(head -c 4000 | tr -d '\0' | tr '\n' ' ')
  # only Japanese text marks a voiceline start; ignores copied URLs, code, etc.
  grep -qP '[\x{3040}-\x{30FF}\x{4E00}-\x{9FFF}]' <<<"$text" || exit 0
  now=$(date +%s.%N)
  # wl-paste --watch replays the pre-existing clipboard once at startup;
  # its timestamp would be wrong, so ignore events right after daemon start
  if [ -f "$START_MARK" ]; then
    started=$(stat -c %Y "$START_MARK")
    awk -v n="$now" -v s="$started" 'BEGIN{exit !(n-s < 1.5)}' && exit 0
  fi
  # Textractor sometimes fires the same line twice back-to-back
  last=$(tail -n 1 "$LINES_LOG" 2>/dev/null)
  if [ -n "$last" ] && [ "${last#*$'\t'}" == "$text" ]; then
    awk -v n="$now" -v p="${last%%$'\t'*}" 'BEGIN{exit !(n-p < 2)}' && exit 0
  fi
  printf '%s\t%s\n' "$now" "$text" >>"$LINES_LOG"
  ;;

run)
  for cmd in wl-paste pactl ffmpeg; do
    if ! command -v "$cmd" &>/dev/null; then
      echo "Error: $cmd is not installed"
      exit 1
    fi
  done

  mkdir -p "$SEGDIR"
  rm -f "$SEGDIR"/seg*.wav
  : >"$LINES_LOG"

  SINK="${VN_SINK:-$(pactl get-default-sink)}"
  if [ -z "$SINK" ]; then
    echo "Error: could not determine default audio sink"
    exit 1
  fi
  echo "Recording ring buffer from ${SINK}.monitor (${SEG_TIME}s x ${SEG_WRAP} segments)"

  touch "$START_MARK"
  wl-paste --type text --watch "$0" _log &
  CLIP_PID=$!
  trap 'kill $CLIP_PID $FF_PID 2>/dev/null' EXIT TERM INT

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
