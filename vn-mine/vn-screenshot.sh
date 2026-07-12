#!/bin/bash
# Visual novel capture → screenshot + Anki upload
# Works perfectly from a KDE shortcut
# Requires: curl, jq, spectacle, notify-send (optional)

ANKI_CONNECT_URL="http://localhost:8765"

# === PREREQUISITE CHECK ===
for cmd in curl jq spectacle; do
  if ! command -v $cmd &>/dev/null; then
    echo "Error: $cmd is not installed"
    exit 1
  fi
done

# === SCREENSHOT ===
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
SCREENSHOT_FILE="screenshot_${TIMESTAMP}.png"

echo "📸 Taking screenshot..."
notify-send "📸 VN Screenshot" "Taking screenshot..."
spectacle -bneo "$SCREENSHOT_FILE" -a

if [ ! -f "$SCREENSHOT_FILE" ]; then
  echo "Error: Failed to take screenshot"
  exit 1
fi
echo "Screenshot saved: $SCREENSHOT_FILE"

# === ANKI SECTION - FIND CARD ===
echo "🔍 Finding most recent Anki card..."
# Find cards with the "Japanese sentences" Note Type added most recently
CARD_IDS=$(curl -s -X POST "$ANKI_CONNECT_URL" -d '{
    "action": "findCards",
    "version": 6,
    "params": { "query": "note:\"Japanese sentences\" added:1" }
}')
MOST_RECENT_CARD=$(echo "$CARD_IDS" | jq -r '.result[-1]')

if [ "$MOST_RECENT_CARD" == "null" ] || [ -z "$MOST_RECENT_CARD" ]; then
  echo "Error: No cards found with note type 'Japanese sentences'"
  notify-send "❌ VN Screenshot" "Error: No recent Anki card found!"
  exit 1
fi
echo "Found card ID: $MOST_RECENT_CARD"

# Get the Note ID associated with the Card ID
NOTE_ID=$(curl -s -X POST "$ANKI_CONNECT_URL" -d "{
    \"action\": \"cardsInfo\",
    \"version\": 6,
    \"params\": { \"cards\": [$MOST_RECENT_CARD] }
}" | jq -r '.result[0].note')

echo "Note ID: $NOTE_ID"

# === UPLOAD MEDIA ===

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

echo "📤 Uploading screenshot to Anki media folder..."
STORE_IMAGE_RESULT=$(curl -s -X POST "$ANKI_CONNECT_URL" -d @"$STORE_IMAGE_PAYLOAD")
rm -f "$STORE_IMAGE_PAYLOAD"

if echo "$STORE_IMAGE_RESULT" | jq -e '.error != null' >/dev/null; then
  echo "Error storing screenshot: $(echo "$STORE_IMAGE_RESULT" | jq -r '.error')"
  notify-send "❌ VN Screenshot" "Error storing media file!"
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
                "Image": "<img src='$SCREENSHOT_FILE'>"
            }
        }
    }
}
EOF

echo "📝 Updating Anki note with image tag..."
UPDATE_RESULT=$(curl -s -X POST "$ANKI_CONNECT_URL" -d @"$UPDATE_PAYLOAD")
rm -f "$UPDATE_PAYLOAD"

if echo "$UPDATE_RESULT" | jq -e '.error != null' >/dev/null; then
  echo "Error updating note: $(echo "$UPDATE_RESULT" | jq -r '.error')"
  notify-send "❌ VN Screenshot" "Error updating note fields!"
  exit 1
fi

# Optional cleanup
rm -f "$SCREENSHOT_FILE"

echo "✅ Successfully added screenshot to Anki card!"
notify-send "✅ VN Screenshot" "Screenshot added to Anki!"
