#!/bin/sh
# Download the latest Kotodex release and set it up.
#
#   curl -fsSL https://raw.githubusercontent.com/geoals/kotodex/master/install.sh | sh
#
# POSIX sh and only curl, tar and sha256sum: this runs before setup.sh has
# checked anything, on a machine that may have none of the rest.
#
# KOTODEX_TARBALL points at a local file or a URL instead of the latest release,
# which is how this is tested without publishing one.
set -eu

REPO="${KOTODEX_REPO:-geoals/kotodex}"
DEST="${KOTODEX_DEST:-$HOME/.local/opt/kotodex}"

bold=''; red=''; off=''
if [ -t 2 ]; then bold=$(printf '\033[1m'); red=$(printf '\033[31m'); off=$(printf '\033[0m'); fi
say()  { printf '%s==>%s %s\n' "$bold" "$off" "$1" >&2; }
die()  { printf '%s✗%s %s\n' "$red" "$off" "$1" >&2; exit 1; }

for bin in curl tar sha256sum; do
  command -v "$bin" >/dev/null 2>&1 || die "$bin is required to install Kotodex"
done

TMP="$(mktemp -d)"
# Leaves nothing behind on a failed download, and the tarball is not wanted
# after it is unpacked.
trap 'rm -rf "$TMP"' EXIT INT TERM

TARBALL="$TMP/kotodex.tar.gz"

if [ -n "${KOTODEX_TARBALL:-}" ]; then
  case "$KOTODEX_TARBALL" in
    http://*|https://*)
      say "downloading $KOTODEX_TARBALL"
      curl -fL --progress-bar -o "$TARBALL" "$KOTODEX_TARBALL" || die "download failed"
      ;;
    *)
      [ -f "$KOTODEX_TARBALL" ] || die "no such file: $KOTODEX_TARBALL"
      say "using $KOTODEX_TARBALL"
      cp "$KOTODEX_TARBALL" "$TARBALL"
      ;;
  esac
else
  say "looking up the latest release of $REPO"
  # sed rather than jq: jq is one of the things setup.sh installs later, so it
  # cannot be a requirement for getting there.
  api="https://api.github.com/repos/$REPO/releases/latest"
  assets="$(curl -fsSL "$api")" || die "could not reach $api"
  url="$(printf '%s' "$assets" \
    | sed -n 's/.*"browser_download_url": *"\([^"]*linux-x86_64\.tar\.gz\)".*/\1/p' \
    | head -1)"
  [ -n "$url" ] || die "no linux-x86_64 tarball in the latest release of $REPO"

  say "downloading $(basename "$url")"
  curl -fL --progress-bar -o "$TARBALL" "$url" || die "download failed"

  # The checksum is published beside the tarball. Absent, the download is still
  # verified by tar refusing to unpack a truncated archive.
  if curl -fsSL -o "$TARBALL.sha256" "$url.sha256" 2>/dev/null; then
    say "verifying"
    expected="$(cut -d' ' -f1 <"$TARBALL.sha256")"
    actual="$(sha256sum "$TARBALL" | cut -d' ' -f1)"
    [ "$expected" = "$actual" ] || die "checksum mismatch — download it again"
  fi
fi

say "unpacking into $DEST"
mkdir -p "$DEST"
# --strip-components=1 drops the version-named top directory, so the same
# destination is reused on an upgrade. Files the tarball does not carry survive
# it: .env with the API key in it, and the dictionary zips.
tar -xzf "$TARBALL" -C "$DEST" --strip-components=1 || die "could not unpack the tarball"

[ -x "$DEST/setup.sh" ] || die "the tarball has no setup.sh"

say "running setup"
# setup.sh reads its prompts from /dev/tty, so it stays interactive even though
# this script arrived on stdin.
cd "$DEST"
exec ./setup.sh "$@"
