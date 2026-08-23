#!/usr/bin/env bash
# Build the release tarball.
#
#   scripts/build-release.sh [version]
#
# Ships the binaries. The reader this is for runs visual novels, not rustup, so
# a tarball that needs a toolchain and a five-minute build is a tarball that
# does not get used.
#
# yt-mine and manga-mine are not in it. They share the language layer and the
# repository and nothing else.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${1:-$(date +%Y.%m.%d)}"
NAME="kotodex-$VERSION-linux-x86_64"
OUT="$REPO/target/release-artifact"
STAGE="$OUT/$NAME"

say() { printf '\033[1m==>\033[0m %s\n' "$1"; }

say "building"
cd "$REPO"
cargo build --release -p read-stats
cargo build --release -p jp-core --bin jp-dict
cargo build --release -p jp-mine-core --bin anki-setup

rm -rf "$STAGE"
# target/release, not bin/: it is where setup.sh, kotodex.py and the doctor
# already look, and one layout for a checkout and a tarball is one thing to be
# wrong about.
mkdir -p "$STAGE/target/release" "$STAGE/dictionaries"

say "collecting"
for bin in read-stats jp-dict anki-setup; do
  cp "$REPO/target/release/$bin" "$STAGE/target/release/$bin"
  strip "$STAGE/target/release/$bin" 2>/dev/null || true
done

copy() { mkdir -p "$STAGE/$(dirname "$1")"; cp -r "$REPO/$1" "$STAGE/$1"; }

copy setup.sh
copy README.md
copy LICENSE
copy THIRD-PARTY.md
copy docs
copy web-shared
copy layer-overlay
copy kotodex
copy read-stats/static
copy read-stats/overlay
copy scripts/lib
copy scripts/kotodex-doctor.sh
copy scripts/start-all.sh

# vn-mine by name rather than wholesale: the directory also holds tests, a
# game-specific script and a stale shim.
mkdir -p "$STAGE/vn-mine"
for f in kotodex-capture kotodex-capture.service vn-capture.sh vn-trim.py vn-vad.py \
         vn-ws-logger.py requirements.txt README.md; do
  cp "$REPO/vn-mine/$f" "$STAGE/vn-mine/$f"
done

find "$STAGE" -name '__pycache__' -type d -prune -exec rm -rf {} +
find "$STAGE" -name '*.pyc' -delete

say "packing"
tar -C "$OUT" -czf "$OUT/$NAME.tar.gz" "$NAME"
sha256sum "$OUT/$NAME.tar.gz" | tee "$OUT/$NAME.tar.gz.sha256"
printf '%s  %s\n' "$(du -h "$OUT/$NAME.tar.gz" | cut -f1)" "$OUT/$NAME.tar.gz"
