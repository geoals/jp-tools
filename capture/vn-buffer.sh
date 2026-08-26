#!/usr/bin/env bash
# Renamed to kotodex-capture. Kept for one release: an installed
# vn-buffer.service still names this path, and a rename must not stop the
# capture daemon of someone who has not reinstalled the unit.
exec "$(dirname "$(readlink -f "$0")")/kotodex-capture" "$@"
