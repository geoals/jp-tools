#!/bin/bash
# VN mine capture — bind this to a single hotkey.
# Cuts the last voiceline out of the vn-buffer ring buffer (start = timestamp
# of the last Japanese line Textractor hooked, end = silero-VAD end of
# speech), screenshots the active window, and attaches both to the most
# recently added "Japanese sentences" Anki note.
# Requires: vn-buffer.service running, curl, jq, spectacle, ffmpeg
# Env: VN_DRY=1        build the clip + screenshot but skip Anki, keep files
#                      (also skips the sentence trim — it needs the note)
#      VN_MAX_LEN=20   max seconds of audio considered after the line appears
#      VN_WHISPER_URL  whisper-service for sentence trim (default :8100)

RUNDIR="${VN_RUNDIR:-${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/vn-mine}"
SEGDIR="$RUNDIR/seg"
LINES_LOG="$RUNDIR/lines.log"
BPS=192000 # 48000 Hz * 2 ch * 2 bytes/sample
WAV_HDR=44 # bytes; vn-buffer.sh records with -fflags +bitexact
PRE_PAD=0.30
POST_PAD=0.25
MAX_LEN="${VN_MAX_LEN:-20}"
ANKI_CONNECT_URL="http://localhost:8765"
WHISPER_URL="${VN_WHISPER_URL:-http://localhost:8100}"
SCRIPT_DIR="$(dirname "$(readlink -f "$0")")"
VAD_PYTHON="$HOME/.local/share/vn-mine/venv/bin/python"
VAD_SCRIPT="$SCRIPT_DIR/vn-vad.py"
TRIM_SCRIPT="$SCRIPT_DIR/vn-trim.py"

TMP=$(mktemp -d "$RUNDIR/cap.XXXXXX" 2>/dev/null) || TMP=$(mktemp -d)

die() {
  echo "Error: $1"
  notify-send -u critical "❌ VN Mine" "$1"
  [ -z "$VN_DRY" ] && rm -rf "$TMP"
  exit 1
}

for cmd in curl jq spectacle ffmpeg; do
  command -v "$cmd" &>/dev/null || die "$cmd is not installed"
done

NOW=$(date +%s.%N)
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# === LOCATE THE VOICELINE START (before the screenshot — anchor the line at
# the press so advancing to the next line immediately after can't re-anchor) ===
[ -s "$LINES_LOG" ] || die "No hooked lines logged yet. Is vn-buffer running and Textractor copying to clipboard?"
LAST_LINE=$(tail -n 1 "$LINES_LOG")
LINE_TS=${LAST_LINE%%$'\t'*}
LINE_TEXT=${LAST_LINE#*$'\t'}

# === SCREENSHOT (capture the window state at the moment of the press) ===
SCREENSHOT_FILE="screenshot_${TIMESTAMP}.png"
spectacle -bneo "$TMP/$SCREENSHOT_FILE" -a
[ -f "$TMP/$SCREENSHOT_FILE" ] || die "Failed to take screenshot"

# Snapshot the ring: fractional mtime + size per segment, oldest first
SEG_SNAPSHOT=$(find "$SEGDIR" -name 'seg*.wav' -printf '%T@ %s %p\n' 2>/dev/null | sort -n)
[ -n "$SEG_SNAPSHOT" ] || die "Ring buffer is empty. Is vn-buffer.service running?"

# The ring is one contiguous PCM stream; anchor its end at the newest
# segment's mtime and work back by byte count to place [START,END] in it.
read -r SKIP_BYTES LEN_BYTES CLIP_START <<<"$(echo "$SEG_SNAPSHOT" | awk \
  -v line_ts="$LINE_TS" -v now="$NOW" -v bps="$BPS" -v hdr="$WAV_HDR" \
  -v pre="$PRE_PAD" -v maxlen="$MAX_LEN" '
  { total += $2 - hdr; last_mtime = $1 }
  END {
    stream_end = last_mtime
    stream_start = stream_end - total / bps
    start = line_ts - pre
    if (start < stream_start) { printf "STALE %.0f %.0f", now - line_ts, total / bps; exit }
    end = start + pre + maxlen
    if (end > stream_end) end = stream_end
    if (end <= start) { print "EMPTY"; exit }
    skip = int((start - stream_start) * bps / 4) * 4
    len  = int((end - start) * bps / 4) * 4
    printf "%d %d %.6f", skip, len, start
  }')"

# on STALE, awk printed "STALE <line-age-s> <ring-coverage-s>" into the next two fields
case "$SKIP_BYTES" in
STALE) die "Last hooked line is ${LEN_BYTES}s old but the ring only holds the last ${CLIP_START}s of audio — press the hotkey sooner after the voiceline plays:
$LINE_TEXT" ;;
EMPTY) die "No audio available after the hooked line yet" ;;
esac

# Concatenate segment payloads (skipping WAV headers) and cut the window
echo "$SEG_SNAPSHOT" | while read -r _ _ f; do tail -c "+$((WAV_HDR + 1))" "$f"; done |
  tail -c "+$((SKIP_BYTES + 1))" | head -c "$LEN_BYTES" >"$TMP/clip.raw"

CLIP_BYTES=$(stat -c %s "$TMP/clip.raw")
[ "$CLIP_BYTES" -ge 19200 ] || die "Extracted clip is too short (${CLIP_BYTES} bytes)"

# === VAD TRIM ===
TRIM_NOTE=""
if [ -x "$VAD_PYTHON" ] && [ -f "$VAD_SCRIPT" ]; then
  ffmpeg -nostdin -loglevel error -f s16le -ar 48000 -ac 2 -i "$TMP/clip.raw" \
    -ac 1 -ar 16000 -c:a pcm_s16le "$TMP/vad.wav" -y
  VAD_OUT=$("$VAD_PYTHON" "$VAD_SCRIPT" "$TMP/vad.wav" 2>"$TMP/vad.err")
  if [ "$VAD_OUT" == "none" ]; then
    TRIM_NOTE=" (⚠ no speech detected — kept full window)"
    notify-send -u critical "⚠️ VN Mine" "No speech detected in the ${MAX_LEN}s after the hooked line — attaching the full window anyway.
Wrong audio output, or did the line hook long after the voice played?"
  elif [ -z "$VAD_OUT" ]; then
    TRIM_NOTE=" (⚠ VAD failed — kept full window)"
    notify-send -u critical "⚠️ VN Mine" "VAD script failed — attaching the untrimmed window.
$(tail -n 1 "$TMP/vad.err" 2>/dev/null)"
  else
    read -r SPEECH_START SPEECH_END <<<"$VAD_OUT"
    read -r TRIM_SKIP TRIM_LEN <<<"$(awk -v s="$SPEECH_START" -v e="$SPEECH_END" \
      -v pre="$PRE_PAD" -v post="$POST_PAD" -v total="$CLIP_BYTES" -v bps="$BPS" 'BEGIN {
        ts = s - pre; if (ts < 0) ts = 0
        te = e + post
        skip = int(ts * bps / 4) * 4
        len = int((te - ts) * bps / 4) * 4
        if (skip + len > total) len = total - skip
        printf "%d %d", skip, len
      }')"
    tail -c "+$((TRIM_SKIP + 1))" "$TMP/clip.raw" | head -c "$TRIM_LEN" >"$TMP/clip2.raw"
    mv "$TMP/clip2.raw" "$TMP/clip.raw"
  fi
else
  TRIM_NOTE=" (VAD unavailable — kept full window)"
fi

# === FIND NEWEST ANKI NOTE (before encode — the sentence trim needs its fields) ===
if [ -z "$VN_DRY" ]; then
  CARD_IDS=$(curl -s -X POST "$ANKI_CONNECT_URL" -d '{
      "action": "findCards",
      "version": 6,
      "params": { "query": "note:\"Japanese sentences\" added:1" }
  }') || die "AnkiConnect is not reachable. Is Anki running?"
  MOST_RECENT_CARD=$(echo "$CARD_IDS" | jq -r '.result[-1]')
  if [ "$MOST_RECENT_CARD" == "null" ] || [ -z "$MOST_RECENT_CARD" ]; then
    die "No cards found with note type 'Japanese sentences'"
  fi

  NOTE_ID=$(curl -s -X POST "$ANKI_CONNECT_URL" -d "{
      \"action\": \"cardsInfo\",
      \"version\": 6,
      \"params\": { \"cards\": [$MOST_RECENT_CARD] }
  }" | jq -r '.result[0].note')
  [ -n "$NOTE_ID" ] && [ "$NOTE_ID" != "null" ] || die "Could not resolve note for card $MOST_RECENT_CARD"

  # === SENTENCE TRIM ===
  # A hooked line can hold several sentences while Yomitan mines just one;
  # cut the clip down to the mined sentence via whisper word timestamps.
  # Any failure (whisper down, no confident match) keeps the VAD-trimmed clip.
  NOTE_FIELDS=$(curl -s -X POST "$ANKI_CONNECT_URL" -d "{
      \"action\": \"notesInfo\",
      \"version\": 6,
      \"params\": { \"notes\": [$NOTE_ID] }
  }" | jq -r '.result[0].fields')
  TARGET_WORD=$(echo "$NOTE_FIELDS" | jq -r '.VocabKanji.value // ""')
  SENTENCE=$(echo "$NOTE_FIELDS" | jq -r '.SentKanji.value // ""')
  if [ -n "$TARGET_WORD" ] && [ -n "$SENTENCE" ] && [ -x "$VAD_PYTHON" ] && [ -f "$TRIM_SCRIPT" ]; then
    ffmpeg -nostdin -loglevel error -f s16le -ar 48000 -ac 2 -i "$TMP/clip.raw" \
      -ac 1 -ar 16000 -c:a pcm_s16le "$TMP/trim.wav" -y
    TRIM_OUT=$("$VAD_PYTHON" "$TRIM_SCRIPT" "$TMP/trim.wav" "$TARGET_WORD" "$SENTENCE" "$WHISPER_URL" 2>"$TMP/trim.err")
    if [[ "$TRIM_OUT" =~ ^[0-9] ]]; then
      read -r SENT_START SENT_END <<<"$TRIM_OUT"
      read -r TRIM_SKIP TRIM_LEN <<<"$(awk -v s="$SENT_START" -v e="$SENT_END" \
        -v total="$(stat -c %s "$TMP/clip.raw")" -v bps="$BPS" 'BEGIN {
          skip = int(s * bps / 4) * 4
          len = int((e - s) * bps / 4) * 4
          if (skip + len > total) len = total - skip
          printf "%d %d", skip, len
        }')"
      tail -c "+$((TRIM_SKIP + 1))" "$TMP/clip.raw" | head -c "$TRIM_LEN" >"$TMP/clip2.raw"
      mv "$TMP/clip2.raw" "$TMP/clip.raw"
      TRIM_NOTE="$TRIM_NOTE ✂"
    fi
  fi
fi

# === ENCODE ===
AUDIO_FILE="recording_${TIMESTAMP}.ogg"
ffmpeg -nostdin -loglevel error -f s16le -ar 48000 -ac 2 -i "$TMP/clip.raw" \
  -c:a libvorbis -q:a 3 "$TMP/$AUDIO_FILE" -y || die "ffmpeg encoding failed"
DURATION=$(awk -v b="$(stat -c %s "$TMP/clip.raw")" -v bps="$BPS" 'BEGIN{printf "%.1f", b/bps}')

if [ -n "$VN_DRY" ]; then
  echo "DRY RUN — no Anki upload"
  echo "Line:      $LINE_TEXT"
  echo "Audio:     $TMP/$AUDIO_FILE (${DURATION}s)$TRIM_NOTE"
  echo "Image:     $TMP/$SCREENSHOT_FILE"
  exit 0
fi

# === UPLOAD MEDIA ===
upload_media() { # filename filepath
  local payload result
  payload=$(mktemp)
  {
    printf '{"action":"storeMediaFile","version":6,"params":{"filename":"%s","data":"' "$1"
    base64 -w 0 "$2"
    printf '"}}'
  } >"$payload"
  result=$(curl -s -X POST "$ANKI_CONNECT_URL" -d @"$payload")
  rm -f "$payload"
  if echo "$result" | jq -e '.error != null' >/dev/null; then
    die "Error storing $1: $(echo "$result" | jq -r '.error')"
  fi
}

upload_media "$SCREENSHOT_FILE" "$TMP/$SCREENSHOT_FILE"
upload_media "$AUDIO_FILE" "$TMP/$AUDIO_FILE"

# === UPDATE NOTE ===
UPDATE_RESULT=$(curl -s -X POST "$ANKI_CONNECT_URL" -d "{
    \"action\": \"updateNoteFields\",
    \"version\": 6,
    \"params\": {
        \"note\": {
            \"id\": $NOTE_ID,
            \"fields\": {
                \"Image\": \"<img src='$SCREENSHOT_FILE'>\",
                \"SentAudio\": \"[sound:$AUDIO_FILE]\"
            }
        }
    }
}")
if echo "$UPDATE_RESULT" | jq -e '.error != null' >/dev/null; then
  die "Error updating note: $(echo "$UPDATE_RESULT" | jq -r '.error')"
fi

rm -rf "$TMP"
echo "✅ Added ${DURATION}s audio + screenshot to note $NOTE_ID"
notify-send "✅ VN Mine" "${DURATION}s audio + screenshot added$TRIM_NOTE
$LINE_TEXT"
