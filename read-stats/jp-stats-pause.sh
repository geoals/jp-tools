#!/usr/bin/env bash
# Toggle the read-stats tracking pause — bind to a hotkey (like vn-capture.sh)
# for skipping scenes without polluting reading stats.
set -euo pipefail

URL="${JP_STATS_URL:-http://localhost:3200}"

if ! out="$(curl -sf -X POST "$URL/api/pause" 2>/dev/null)"; then
  notify-send -u critical -a read-stats "read-stats" "toggle failed — is read-stats running?"
  exit 1
fi

if [[ "$(jq -r .paused <<<"$out")" == "true" ]]; then
  notify-send -a read-stats -t 2500 "read-stats" "⏸ tracking paused"
else
  notify-send -a read-stats -t 2500 "read-stats" "▶ tracking resumed"
fi
