#!/usr/bin/env bash
# T0.4: can xdotool still address the game window in this session?
# Usage: geometry-test.sh <window name substring>
set -u
name="${1:?usage: geometry-test.sh <window name substring>}"
wid=$(xdotool search --name "$name" 2>/dev/null | head -n1)
[ -n "$wid" ] || { echo "no window matching '$name'"; exit 1; }
xdotool getwindowgeometry "$wid"
