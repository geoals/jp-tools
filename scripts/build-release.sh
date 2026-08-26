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
cargo build --release -p kotodex-server
cargo build --release -p jp-core --bin jp-dict
cargo build --release -p jp-mine-core --bin anki-setup

rm -rf "$STAGE"
# target/release, not bin/: it is where setup.sh, kotodex.py and the doctor
# already look, and one layout for a checkout and a tarball is one thing to be
# wrong about.
mkdir -p "$STAGE/target/release" "$STAGE/dictionaries"

say "collecting"
for bin in kotodex-server jp-dict anki-setup; do
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
copy kotodex-server/static
copy kotodex-server/overlay
copy scripts/lib
copy scripts/kotodex-doctor.sh
# start-all.sh is deliberately absent. It manages yt-mine, manga-mine,
# whisper-service and the OCR service as well, none of which is in here, so in a
# tarball it is four failures for things that were never shipped. The launcher
# runs kotodex-server itself.

# By name rather than wholesale: both directories also hold tests.
mkdir -p "$STAGE/capture" "$STAGE/sources/textractor"
for f in kotodex-capture kotodex-capture.service vn-capture.sh vn-trim.py vn-vad.py \
         requirements.txt README.md; do
  cp "$REPO/capture/$f" "$STAGE/capture/$f"
done
cp "$REPO/sources/textractor/vn-ws-logger.py" "$STAGE/sources/textractor/"
cp "$REPO/sources/README.md" "$STAGE/sources/README.md"

find "$STAGE" -name '__pycache__' -type d -prune -exec rm -rf {} +
find "$STAGE" -name '*.pyc' -delete

say "packing"
tar -C "$OUT" -czf "$OUT/$NAME.tar.gz" "$NAME"
# From $OUT, so the file names the tarball and not this machine's paths:
# `sha256sum -c` runs next to the download.
(cd "$OUT" && sha256sum "$NAME.tar.gz" | tee "$NAME.tar.gz.sha256")
printf '%s  %s\n' "$(du -h "$OUT/$NAME.tar.gz" | cut -f1)" "$OUT/$NAME.tar.gz"
