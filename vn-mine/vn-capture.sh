#!/bin/bash
# VN mine capture — bind this to a single hotkey.
# Cuts the last voiceline out of the kotodex-capture ring buffer (start = timestamp
# of the last Japanese line Textractor hooked, end = silero-VAD end of speech,
# never past the *next* hooked line), screenshots the active window, and
# attaches both to the most recently added note of the configured note type.
# Requires: kotodex-capture running, curl, jq, spectacle, ffmpeg
# Env: VN_DRY=1        build the clip + screenshot but skip Anki, keep files
#                      (also skips the sentence trim — it needs the note)
#      VN_JSON=1       print a JSON result object instead of notifying the
#                      desktop — for read-stats' reader view, which shows the
#                      result in the browser that mined
#      VN_MAX_LEN=10   max seconds of audio considered after the line appears
#      VN_ANCHOR_TS    epoch seconds; anchor on the newest hooked line at *that*
#                      instant rather than at the moment this script runs. What
#                      read-stats passes when a card add triggers the capture,
#                      so reading on while the capture works can't move the
#                      anchor onto the next line.
#      VN_NOTE_ID      attach to this note instead of the most recently added
#                      one — again for the card-add path, which already knows
#                      which note it just created.
#      VN_MIN_PEAK_DB  reject a clip whose peak is quieter than this (default
#                      -25 dBFS — a silence floor, not a voice test)
#      VN_WHISPER_URL  whisper-service for sentence trim (default :8100)
#      VN_WINDOW       name (substring) of the VN's window — capture it by id
#                      instead of whatever has focus. Needed when mining from
#                      read-stats' #read page, where the browser is focused.
#                      Unset, it is asked for: read-stats holds it per work.
#      JP_TOOLS_READ_STATS_URL  where to ask (default http://localhost:3200)

RUNDIR="${VN_RUNDIR:-${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/vn-mine}"
SEGDIR="$RUNDIR/seg"
LINES_LOG="$RUNDIR/lines.log"
BPS=192000 # 48000 Hz * 2 ch * 2 bytes/sample
WAV_HDR=44 # bytes; kotodex-capture records with -fflags +bitexact
PRE_PAD=0.30
POST_PAD=0.25
# 10s, not 20: a voiceline that long doesn't exist, so the extra ten seconds
# only ever widen the window a false positive can be found in.
MAX_LEN="${VN_MAX_LEN:-10}"
# Shortest window worth running VAD over, once the next hooked line has bounded
# it. Below this the reader advanced almost at once, so there was no voiceline
# to hear — or it was not waited for — and whatever is in the ring from there on
# is the next line's.
MIN_LEN="${VN_MIN_LEN:-0.6}"
MIN_PEAK_DB="${VN_MIN_PEAK_DB:--25}"
SCRIPT_DIR="$(dirname "$(readlink -f "$0")")"
# The field names live in the repo's .env, which the services load through
# dotenvy and inherit to this script. Run from a hotkey there is no parent to
# inherit from, so read it here too — same file, so one answer either way.
REPO_ENV="$SCRIPT_DIR/../.env"
if [ -f "$REPO_ENV" ]; then
  while IFS= read -r line; do
    case "$line" in
      JP_TOOLS_ANKI_*=*)
        name="${line%%=*}"
        [ -n "${!name+set}" ] && continue
        value="${line#*=}"
        value="${value%\"}"
        value="${value#\"}"
        export "$name=$value"
        ;;
    esac
  done <"$REPO_ENV"
fi
ANKI_CONNECT_URL="${JP_TOOLS_ANKI_URL:-http://127.0.0.1:8765}"
WHISPER_URL="${VN_WHISPER_URL:-http://localhost:8100}"
VAD_PYTHON="$HOME/.local/share/vn-mine/venv/bin/python"
VAD_SCRIPT="$SCRIPT_DIR/vn-vad.py"
TRIM_SCRIPT="$SCRIPT_DIR/vn-trim.py"
VN_WINDOW="${VN_WINDOW:-}"
VN_ANCHOR_TS="${VN_ANCHOR_TS:-}"
VN_NOTE_ID="${VN_NOTE_ID:-}"
SHOT_NOTE=""

READ_STATS_URL="${JP_TOOLS_READ_STATS_URL:-http://localhost:3200}"

TMP=$(mktemp -d "$RUNDIR/cap.XXXXXX" 2>/dev/null) || TMP=$(mktemp -d)

die() {
  if [ -n "$VN_JSON" ]; then
    # jq is checked for below, but this runs before that — fall back to a
    # hand-built object so the caller always gets a parseable failure.
    jq -nc --arg error "$1" '{ok: false, error: $error}' 2>/dev/null ||
      printf '{"ok":false,"error":"vn-capture failed early (is jq installed?)"}\n'
  else
    echo "Error: $1"
    notify-send -u critical "❌ VN Mine" "$1"
  fi
  [ -z "$VN_DRY" ] && rm -rf "$TMP"
  exit 1
}

for cmd in curl jq spectacle ffmpeg; do
  command -v "$cmd" &>/dev/null || die "$cmd is not installed"
done

# Unset — fired by hotkey rather than by read-stats — so ask read-stats which
# window is the VN, and aim at the same one the mine button does. Resolving it
# here in SQL instead made this a second implementation of a rule that must have
# exactly one: switching VNs would mean updating two places, and the one you
# forget silently captures the last game.
#
# No answer means no window, and the screenshot falls back to whatever has
# focus. That is what read-stats being down looks like, and it beats failing a
# capture over it.
if [ -z "$VN_WINDOW" ]; then
  VN_WINDOW=$(curl -s --max-time 2 "$READ_STATS_URL/api/vn/window" 2>/dev/null |
    jq -r '.window // empty' 2>/dev/null)
fi

NOW=$(date +%s.%N)
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# The note type's field names, the same variables the Rust side reads so one
# note type is described in one place. The defaults are Lapis's.
ANKI_MODEL="${JP_TOOLS_ANKI_MODEL:-Lapis}"
FIELD_VOCAB="${JP_TOOLS_ANKI_FIELD_VOCAB:-Expression}"
FIELD_SENTENCE="${JP_TOOLS_ANKI_FIELD_SENTENCE:-Sentence}"
FIELD_IMAGE="${JP_TOOLS_ANKI_FIELD_IMAGE:-Picture}"
FIELD_AUDIO="${JP_TOOLS_ANKI_FIELD_AUDIO:-SentenceAudio}"

# === LOCATE THE VOICELINE START (before the screenshot — anchor the line at
# the press so advancing to the next line immediately after can't re-anchor) ===
[ -s "$LINES_LOG" ] || die "No hooked lines logged yet. Is kotodex-capture running and Textractor copying to clipboard?"
# With VN_ANCHOR_TS, the newest line *as of that instant* — the line that was on
# screen when the card was added, not whatever is on screen now. The card-add
# path can take seconds to reach here (an LLM call, a screenshot, VAD), which is
# long enough to read on, and then `tail -n 1` anchors the audio and the
# screenshot on the following line. LC_ALL=C so a comma-decimal locale doesn't
# truncate the timestamps to whole seconds when comparing.
LAST_LINE=""
if [ -n "$VN_ANCHOR_TS" ]; then
  LAST_LINE=$(LC_ALL=C awk -F'\t' -v a="$VN_ANCHOR_TS" '$1 <= a { l = $0 } END { print l }' "$LINES_LOG")
fi
# No anchor, or an anchor older than everything the log still holds: the newest
# line is the best guess left, which is also the hotkey's own behaviour.
[ -n "$LAST_LINE" ] || LAST_LINE=$(tail -n 1 "$LINES_LOG")
LINE_TS=${LAST_LINE%%$'\t'*}
LINE_TEXT=${LAST_LINE#*$'\t'}

# The line *after* the anchor, if one has been hooked since. Whatever plays
# from that moment on belongs to that line, not to this one, so it bounds the
# search window — otherwise the window runs MAX_LEN seconds forward through
# however many lines were advanced past, and the first voice it finds wins. An
# unvoiced line mined just before a voiced one is the sharp case: its own span
# holds no speech, so the clip becomes the next line's voice entirely.
# Empty when the anchor is still the newest line, which is the hotkey's normal
# case and leaves the window unbounded as before.
NEXT_TS=$(LC_ALL=C awk -F'\t' -v t="$LINE_TS" '$1 > t { print $1; exit }' "$LINES_LOG")

# === SCREENSHOT (capture the window state at the moment of the press) ===
# `spectacle -a` grabs whatever has focus, which is only the VN when the hotkey
# was pressed with the VN focused. Mining from read-stats' #read page focuses
# the browser instead, so the default would capture the browser.
#
# VN_WINDOW sidesteps focus entirely: find the VN's window by name and grab it
# by id. Wine/Proton windows are XWayland, so xdotool can still address them
# under a Wayland session even though `xdotool getactivewindow` can't. Any
# failure falls through to the old active-window path rather than dying — a
# wrong screenshot is recoverable, a lost voiceline is not.
SCREENSHOT_FILE="screenshot_${TIMESTAMP}.png"
VN_WID=""
# Which X display the game is on, rather than whichever one this process
# inherited: a service started before the session, or from another session, gets
# a stale DISPLAY and then finds no window at all. Search each one and keep the
# display the window was actually found on.
# Prints "<display> <window id>". Both, because the display is the finding: an
# `export` inside the command substitution that reads this would not reach the
# caller, and grabbing the window needs the display it was found on.
find_vn_window() {
  local d wid
  for d in "${DISPLAY:-}" :0 :1 :2; do
    [ -n "$d" ] || continue
    wid=$(DISPLAY="$d" xdotool search --name "$VN_WINDOW" 2>/dev/null | head -n 1)
    [ -n "$wid" ] && { echo "$d $wid"; return 0; }
  done
  return 1
}

if [ -n "$VN_WINDOW" ]; then
  if command -v xdotool &>/dev/null && command -v import &>/dev/null; then
    read -r VN_DISPLAY VN_WID <<<"$(find_vn_window)"
    [ -n "$VN_DISPLAY" ] && export DISPLAY="$VN_DISPLAY"
    # -silent: import rings the X bell around every capture, which Plasma turns
    # into an audible system beep — one per mine, while reading.
    [ -n "$VN_WID" ] && import -silent -window "$VN_WID" "$TMP/$SCREENSHOT_FILE" 2>/dev/null
    if [ ! -s "$TMP/$SCREENSHOT_FILE" ]; then
      SHOT_NOTE=" (⚠ no window matching '$VN_WINDOW' — captured the whole screen)"
    fi
  else
    SHOT_NOTE=" (⚠ VN_WINDOW set but xdotool/import missing — captured the focused window)"
  fi
else
  SHOT_NOTE=" (⚠ no window name on this work — captured the focused window)"
fi
# The X root before spectacle. The overlay is a Wayland layer surface, so an X
# grab cannot see it while spectacle's "active window" *is* it whenever the mine
# came from the overlay — the fallback would capture the reader, never the game.
if [ ! -s "$TMP/$SCREENSHOT_FILE" ] && command -v import &>/dev/null; then
  rm -f "$TMP/$SCREENSHOT_FILE"
  import -silent -window root "$TMP/$SCREENSHOT_FILE" 2>/dev/null
fi
if [ ! -s "$TMP/$SCREENSHOT_FILE" ]; then
  rm -f "$TMP/$SCREENSHOT_FILE"
  spectacle -bneo "$TMP/$SCREENSHOT_FILE" -a
fi
[ -s "$TMP/$SCREENSHOT_FILE" ] || die "Failed to take screenshot"

# Snapshot the ring: fractional mtime + size per segment, oldest first
SEG_SNAPSHOT=$(find "$SEGDIR" -name 'seg*.wav' -printf '%T@ %s %p\n' 2>/dev/null | sort -n)
[ -n "$SEG_SNAPSHOT" ] || die "Ring buffer is empty. Is kotodex-capture running?"

# The ring is one contiguous PCM stream; anchor its end at the newest
# segment's mtime and work back by byte count to place [START,END] in it.
# LC_ALL=C: the timestamps are fractional, and a comma-decimal locale reads
# them as whole seconds — which silently rounds the window to the wrong second.
read -r SKIP_BYTES LEN_BYTES CLIP_START <<<"$(echo "$SEG_SNAPSHOT" | LC_ALL=C awk \
  -v line_ts="$LINE_TS" -v next_ts="$NEXT_TS" -v now="$NOW" -v bps="$BPS" -v hdr="$WAV_HDR" \
  -v pre="$PRE_PAD" -v maxlen="$MAX_LEN" -v minlen="$MIN_LEN" '
  { total += $2 - hdr; last_mtime = $1 }
  END {
    stream_end = last_mtime
    stream_start = stream_end - total / bps
    start = line_ts - pre
    if (start < stream_start) { printf "STALE %.0f %.0f", now - line_ts, total / bps; exit }
    end = start + pre + maxlen
    if (end > stream_end) end = stream_end
    # Nothing from the next line onwards can be this line’s voice.
    if (next_ts != "" && end > next_ts) end = next_ts
    if (end <= start) { print "EMPTY"; exit }
    # Bounded so tightly there is no room for a voiceline: the reader advanced
    # almost immediately, so this line either was not voiced or was not heard.
    # Screenshot-only rather than an error — the card still wants its picture.
    # Measured from the line, not from the padded window start: the pre-pad is
    # audio from *before* the line and can never hold its voice.
    if (end - line_ts < minlen) { printf "SHORT %.2f", end - line_ts; exit }
    skip = int((start - stream_start) * bps / 4) * 4
    len  = int((end - start) * bps / 4) * 4
    printf "%d %d %.6f", skip, len, start
  }')"

# on STALE, awk printed "STALE <line-age-s> <ring-coverage-s>" into the next two fields
NO_ROOM=""
case "$SKIP_BYTES" in
STALE) die "Last hooked line is ${LEN_BYTES}s old but the ring only holds the last ${CLIP_START}s of audio — press the hotkey sooner after the voiceline plays:
$LINE_TEXT" ;;
EMPTY) die "No audio available after the hooked line yet" ;;
SHORT) NO_ROOM="${LEN_BYTES}s" ;;
esac

# === VAD TRIM ===
# NO_AUDIO: VAD is confident there was no voice at all (an unvoiced line, a
# narration-only screen). Attaching the raw window there would put ${MAX_LEN}s
# of room tone on the card, so the capture becomes screenshot-only. A VAD
# *failure* is different — nothing is known about the audio, so the untrimmed
# window is still the best guess and gets attached as before.
TRIM_NOTE=""
NO_AUDIO=""

# 16 kHz mono WAV of the current clip — what both VAD passes want.
vad_wav() { # out.wav
  ffmpeg -nostdin -loglevel error -f s16le -ar 48000 -ac 2 -i "$TMP/clip.raw" \
    -ac 1 -ar 16000 -c:a pcm_s16le "$1" -y
}

# Give up on the audio and keep the screenshot. $1 is the short reason for the
# card note, $2 the longer one for the desktop; in JSON mode the note is all
# that comes back, since nobody is looking at this desktop.
drop_audio() { # short long
  NO_AUDIO=1
  TRIM_NOTE=" ($1 — screenshot only)"
  [ -z "$VN_JSON" ] && notify-send "⚠️ VN Mine" "$2
If the line was voiced, check the audio output or press sooner after it plays."
}

# The next line arrived too soon for anything in between to be this line's
# voice. Nothing to extract, nothing to trim — take the screenshot and go.
if [ -n "$NO_ROOM" ]; then
  drop_audio "next line ${NO_ROOM} later" \
    "The next line was hooked ${NO_ROOM} after this one, which leaves no room for a voiceline — anything playing by then belongs to that line."
fi

# Concatenate segment payloads (skipping WAV headers) and cut the window
if [ -z "$NO_AUDIO" ]; then
  echo "$SEG_SNAPSHOT" | while read -r _ _ f; do tail -c "+$((WAV_HDR + 1))" "$f"; done |
    tail -c "+$((SKIP_BYTES + 1))" | head -c "$LEN_BYTES" >"$TMP/clip.raw"

  CLIP_BYTES=$(stat -c %s "$TMP/clip.raw")
  [ "$CLIP_BYTES" -ge 19200 ] || die "Extracted clip is too short (${CLIP_BYTES} bytes)"
fi

if [ -n "$NO_AUDIO" ]; then
  : # already decided against audio above — skip VAD entirely
elif [ -x "$VAD_PYTHON" ] && [ -f "$VAD_SCRIPT" ]; then
  vad_wav "$TMP/vad.wav"
  VAD_OUT=$("$VAD_PYTHON" "$VAD_SCRIPT" "$TMP/vad.wav" 2>"$TMP/vad.err")
  if [ "$VAD_OUT" == "none" ]; then
    drop_audio "no speech detected" \
      "No speech detected in the ${MAX_LEN}s after the hooked line."
  elif [ -z "$VAD_OUT" ]; then
    TRIM_NOTE=" (⚠ VAD failed — kept full window)"
    [ -z "$VN_JSON" ] && notify-send -u critical "⚠️ VN Mine" "VAD script failed — attaching the untrimmed window.
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

    # === CONFIRM THE TRIM ===
    # Two checks on what actually came out, because the pass above judged it
    # inside the whole window and both failure modes only show up once the
    # clip stands alone.
    #
    # Silence floor: a clip that never rises above MIN_PEAK_DB is room tone.
    # This does not separate voice from sound effects — measured over a day of
    # mining, real voicelines peak between -6 and -16 dBFS and the sound effect
    # that prompted this peaked at -15 — it only catches a window that holds
    # nothing at all.
    PEAK_DB=$(ffmpeg -nostdin -f s16le -ar 48000 -ac 2 -i "$TMP/clip.raw" \
      -af volumedetect -f null - 2>&1 | grep -oE 'max_volume: -?[0-9.]+' | grep -oE '\-?[0-9.]+$')
    if [ -n "$PEAK_DB" ] && LC_ALL=C awk -v p="$PEAK_DB" -v min="$MIN_PEAK_DB" \
      'BEGIN { exit !(p < min) }'; then
      drop_audio "clip is silent (${PEAK_DB} dBFS)" \
        "The ${MAX_LEN}s after the hooked line peak at only ${PEAK_DB} dBFS — there is nothing on the clip."
    else
      # Cold re-run. silero is stateful, so a loud transient partway through a
      # long window can cross the threshold on state warmed by the preceding
      # audio; scored on its own from a zero state the same clip does not.
      # That is the check that actually separates the two — over the same day's
      # clips, every voiced one peaks at ~1.00 confidence standalone and the
      # sound effect reached 0.35.
      vad_wav "$TMP/vad2.wav"
      if [ "$("$VAD_PYTHON" "$VAD_SCRIPT" "$TMP/vad2.wav" 2>/dev/null)" == "none" ]; then
        drop_audio "no speech on its own" \
          "What VAD found in the ${MAX_LEN}s after the hooked line does not read as speech on its own — a sound effect, most likely."
      fi
    fi
  fi
else
  TRIM_NOTE=" (VAD unavailable — kept full window)"
fi

# === FIND NEWEST ANKI NOTE (before encode — the sentence trim needs its fields) ===
if [ -z "$VN_DRY" ]; then
  # VN_NOTE_ID: the caller created the note and knows its id, so don't go
  # looking for "the most recently added one" — that answer is only right while
  # no other card is added in between.
  if [ -n "$VN_NOTE_ID" ]; then
    NOTE_ID="$VN_NOTE_ID"
  else
    CARD_IDS=$(curl -s -X POST "$ANKI_CONNECT_URL" -d '{
        "action": "findCards",
        "version": 6,
        "params": { "query": "note:\"'"$ANKI_MODEL"'\" added:1" }
    }') || die "AnkiConnect is not reachable. Is Anki running?"
    MOST_RECENT_CARD=$(echo "$CARD_IDS" | jq -r '.result[-1]')
    if [ "$MOST_RECENT_CARD" == "null" ] || [ -z "$MOST_RECENT_CARD" ]; then
      die "No cards found with note type '$ANKI_MODEL'"
    fi

    NOTE_ID=$(curl -s -X POST "$ANKI_CONNECT_URL" -d "{
        \"action\": \"cardsInfo\",
        \"version\": 6,
        \"params\": { \"cards\": [$MOST_RECENT_CARD] }
    }" | jq -r '.result[0].note')
    [ -n "$NOTE_ID" ] && [ "$NOTE_ID" != "null" ] || die "Could not resolve note for card $MOST_RECENT_CARD"
  fi

  # === SENTENCE TRIM ===
  # A hooked line can hold several sentences while Yomitan mines just one;
  # cut the clip down to the mined sentence via whisper word timestamps.
  # Any failure (whisper down, no confident match) keeps the VAD-trimmed clip.
  NOTE_FIELDS=$(curl -s -X POST "$ANKI_CONNECT_URL" -d "{
      \"action\": \"notesInfo\",
      \"version\": 6,
      \"params\": { \"notes\": [$NOTE_ID] }
  }" | jq -r '.result[0].fields')
  TARGET_WORD=$(echo "$NOTE_FIELDS" | jq -r --arg f "$FIELD_VOCAB" '.[$f].value // ""')
  SENTENCE=$(echo "$NOTE_FIELDS" | jq -r --arg f "$FIELD_SENTENCE" '.[$f].value // ""')
  if [ -z "$NO_AUDIO" ] && [ -n "$TARGET_WORD" ] && [ -n "$SENTENCE" ] && [ -x "$VAD_PYTHON" ] && [ -f "$TRIM_SCRIPT" ]; then
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
AUDIO_FILE=""
DURATION=""
if [ -z "$NO_AUDIO" ]; then
  AUDIO_FILE="recording_${TIMESTAMP}.ogg"
  ffmpeg -nostdin -loglevel error -f s16le -ar 48000 -ac 2 -i "$TMP/clip.raw" \
    -c:a libvorbis -q:a 3 "$TMP/$AUDIO_FILE" -y || die "ffmpeg encoding failed"
  # LC_ALL=C: a comma-decimal locale would print "2,4" here, which is not valid
  # JSON for the VN_JSON result below.
  DURATION=$(LC_ALL=C awk -v b="$(stat -c %s "$TMP/clip.raw")" -v bps="$BPS" 'BEGIN{printf "%.1f", b/bps}')
fi

# Everything worth telling the user about this capture, in one string.
NOTE="$TRIM_NOTE$SHOT_NOTE"

if [ -n "$VN_DRY" ]; then
  echo "DRY RUN — no Anki upload"
  echo "Line:      $LINE_TEXT"
  if [ -n "$NO_AUDIO" ]; then
    echo "Audio:     none$TRIM_NOTE"
  else
    echo "Audio:     $TMP/$AUDIO_FILE (${DURATION}s)$TRIM_NOTE"
  fi
  echo "Image:     $TMP/$SCREENSHOT_FILE${VN_WID:+ (window $VN_WID)}$SHOT_NOTE"
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
[ -z "$NO_AUDIO" ] && upload_media "$AUDIO_FILE" "$TMP/$AUDIO_FILE"

# === UPDATE NOTE ===
# The audio field is left out entirely when there was no speech, rather than set
# to an empty string: whatever the note already holds is better than nothing.
FIELDS=$(jq -nc --arg img "<img src='$SCREENSHOT_FILE'>" \
  --arg audio "${AUDIO_FILE:+[sound:$AUDIO_FILE]}" \
  --arg fimg "$FIELD_IMAGE" --arg faud "$FIELD_AUDIO" \
  '{($fimg): $img} + (if $audio == "" then {} else {($faud): $audio} end)')
UPDATE_RESULT=$(curl -s -X POST "$ANKI_CONNECT_URL" -d "{
    \"action\": \"updateNoteFields\",
    \"version\": 6,
    \"params\": {
        \"note\": {
            \"id\": $NOTE_ID,
            \"fields\": $FIELDS
        }
    }
}")
if echo "$UPDATE_RESULT" | jq -e '.error != null' >/dev/null; then
  die "Error updating note: $(echo "$UPDATE_RESULT" | jq -r '.error')"
fi

rm -rf "$TMP"
if [ -n "$VN_JSON" ]; then
  # duration is null on a screenshot-only capture — the reader keys its wording
  # off that rather than off the note text.
  jq -nc --argjson note_id "$NOTE_ID" --argjson duration "${DURATION:-null}" \
    --arg note "$NOTE" --arg line "$LINE_TEXT" \
    '{ok: true, note_id: $note_id, duration: $duration, note: ($note | ltrimstr(" ")), line: $line}'
else
  WHAT="${DURATION:+${DURATION}s audio + }screenshot"
  echo "✅ Added $WHAT to note $NOTE_ID"
  notify-send "✅ VN Mine" "$WHAT added$NOTE
$LINE_TEXT"
fi
