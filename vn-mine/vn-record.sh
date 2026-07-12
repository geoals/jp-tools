#!/bin/bash
# Visual novel capture → screenshot + replay + record + Anki upload
# Toggle behavior: first run starts recording, second run stops it
# Works perfectly from a KDE shortcut
# Requires: ydotool, pw-record, ffmpeg, curl, jq, spectacle, notify-send (optional)

STATE_FILE="/tmp/vn_record_state.pid"
AUDIO_DURATION=8 # seconds, safety timeout
ANKI_CONNECT_URL="http://localhost:8765"

# === TOGGLE LOGIC ===
if [ -f "$STATE_FILE" ]; then
  echo "🛑 Stopping recording..."
  RECORD_PID=$(cat "$STATE_FILE")
  if kill "$RECORD_PID" 2>/dev/null; then
    rm -f "$STATE_FILE"
    notify-send "🛑 VN Recorder" "Recording stopped"
    echo "Recording stopped."
    exit 0
  else
    echo "Warning: could not stop recording process $RECORD_PID."
    rm -f "$STATE_FILE"
    exit 1
  fi
fi

# === START RECORDING ===
for cmd in ydotool pw-record ffmpeg curl jq spectacle; do
  if ! command -v $cmd &>/dev/null; then
    echo "Error: $cmd is not installed"
    exit 1
  fi
done

if ! pgrep -x ydotoold >/dev/null; then
  echo "Error: ydotoold daemon is not running"
  echo "Start it with: sudo systemctl start ydotool"
  exit 1
fi

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
SCREENSHOT_FILE="screenshot_${TIMESTAMP}.png"
TEMP_WAV="temp_${TIMESTAMP}.wav"
AUDIO_FILE="recording_${TIMESTAMP}.ogg"

echo "📸 Taking screenshot..."
spectacle -bneo "$SCREENSHOT_FILE" -a

if [ ! -f "$SCREENSHOT_FILE" ]; then
  echo "Error: Failed to take screenshot"
  exit 1
fi
echo "Screenshot saved: $SCREENSHOT_FILE"

echo "⌨️  Pressing right arrow key..."
ydotool key 106:1 106:0

echo "🎙️ Starting recording (run again to stop, or auto-stops after ${AUDIO_DURATION}s)..."
notify-send "🎙️ VN Recorder" "Recording started"

timeout --foreground "$AUDIO_DURATION" pw-record -P '{ stream.capture.sink=true }' "$TEMP_WAV" &
RECORD_PID=$!
echo $RECORD_PID >"$STATE_FILE"

# Wait until process ends naturally or user stops it
wait $RECORD_PID 2>/dev/null
rm -f "$STATE_FILE"

echo "🔄 Converting to Ogg Vorbis..."
ffmpeg -i "$TEMP_WAV" -c:a libvorbis -q:a 3 "$AUDIO_FILE" -y 2>/dev/null
rm -f "$TEMP_WAV"

echo "Audio saved: $AUDIO_FILE"
notify-send "🎧 VN Recorder" "Recording saved"

# === ANKI SECTION ===
echo "🔍 Finding most recent Anki card..."
CARD_IDS=$(curl -s -X POST "$ANKI_CONNECT_URL" -d '{
    "action": "findCards",
    "version": 6,
    "params": { "query": "note:\"Japanese sentences\" added:1" }
}')
MOST_RECENT_CARD=$(echo "$CARD_IDS" | jq -r '.result[-1]')

if [ "$MOST_RECENT_CARD" == "null" ] || [ -z "$MOST_RECENT_CARD" ]; then
  echo "Error: No cards found with note type 'Japanese sentences'"
  exit 1
fi
echo "Found card ID: $MOST_RECENT_CARD"

NOTE_ID=$(curl -s -X POST "$ANKI_CONNECT_URL" -d "{
    \"action\": \"cardsInfo\",
    \"version\": 6,
    \"params\": { \"cards\": [$MOST_RECENT_CARD] }
}" | jq -r '.result[0].note')

echo "Note ID: $NOTE_ID"

# === UPLOAD MEDIA SAFELY ===

# Upload screenshot
SCREENSHOT_BASE64=$(base64 -w 0 "$SCREENSHOT_FILE")
STORE_IMAGE_PAYLOAD=$(mktemp)
cat >"$STORE_IMAGE_PAYLOAD" <<EOF
{
    "action": "storeMediaFile",
    "version": 6,
    "params": {
        "filename": "$SCREENSHOT_FILE",
        "data": "$SCREENSHOT_BASE64"
    }
}
EOF

STORE_IMAGE_RESULT=$(curl -s -X POST "$ANKI_CONNECT_URL" -d @"$STORE_IMAGE_PAYLOAD")
rm -f "$STORE_IMAGE_PAYLOAD"

if echo "$STORE_IMAGE_RESULT" | jq -e '.error != null' >/dev/null; then
  echo "Error storing screenshot: $(echo "$STORE_IMAGE_RESULT" | jq -r '.error')"
  exit 1
fi

# Upload audio
AUDIO_BASE64=$(base64 -w 0 "$AUDIO_FILE")
STORE_AUDIO_PAYLOAD=$(mktemp)
cat >"$STORE_AUDIO_PAYLOAD" <<EOF
{
    "action": "storeMediaFile",
    "version": 6,
    "params": {
        "filename": "$AUDIO_FILE",
        "data": "$AUDIO_BASE64"
    }
}
EOF

STORE_AUDIO_RESULT=$(curl -s -X POST "$ANKI_CONNECT_URL" -d @"$STORE_AUDIO_PAYLOAD")
rm -f "$STORE_AUDIO_PAYLOAD"

if echo "$STORE_AUDIO_RESULT" | jq -e '.error != null' >/dev/null; then
  echo "Error storing audio: $(echo "$STORE_AUDIO_RESULT" | jq -r '.error')"
  exit 1
fi

# === UPDATE ANKI NOTE ===
UPDATE_PAYLOAD=$(mktemp)
cat >"$UPDATE_PAYLOAD" <<EOF
{
    "action": "updateNoteFields",
    "version": 6,
    "params": {
        "note": {
            "id": $NOTE_ID,
            "fields": {
                "Image": "<img src='$SCREENSHOT_FILE'>",
                "SentAudio": "[sound:$AUDIO_FILE]"
            }
        }
    }
}
EOF

UPDATE_RESULT=$(curl -s -X POST "$ANKI_CONNECT_URL" -d @"$UPDATE_PAYLOAD")
rm -f "$UPDATE_PAYLOAD"

if echo "$UPDATE_RESULT" | jq -e '.error != null' >/dev/null; then
  echo "Error updating note: $(echo "$UPDATE_RESULT" | jq -r '.error')"
  exit 1
fi

# Optional cleanup
rm -f "$SCREENSHOT_FILE" "$AUDIO_FILE"

echo "✅ Successfully added screenshot and audio to Anki card!"
notify-send "✅ VN Recorder" "Screenshot and audio added to Anki!"
