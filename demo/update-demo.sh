#!/bin/sh
# Redeploy the demo when a new release is published.
#
# Runs on the NAS from cron. It polls rather than being pushed to from CI
# because the host is on a home LAN: a GitHub Actions deploy would need inbound
# access to it, and a public repository holding the key for that.
#
# The demo runs the release tarball people actually download, so what is
# deployed here cannot drift from what is shipped.
set -eu

STACK="$(cd "$(dirname "$0")" && pwd)"
REPO="${KOTODEX_REPO:-geoals/kotodex}"
# The name the Dockerfile's ARG TARBALL defaults to.
TARBALL="$STACK/build/kotodex-demo-linux-x86_64.tar.gz"
STAMP="$STACK/deployed-tag"

log() { printf '%s %s\n' "$(date -Is)" "$1"; }
die() { log "error: $1"; exit 1; }

# cron will start another run while a slow build is still going; the second one
# would rebuild from a half-written tarball.
exec 9>"$STACK/.update.lock"
flock -n 9 || { log "another run is in progress"; exit 0; }

release="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest")" \
  || die "could not reach the GitHub API"
tag="$(printf '%s' "$release" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)"
[ -n "$tag" ] || die "no tag_name in the API response"

[ -f "$STAMP" ] && [ "$(cat "$STAMP")" = "$tag" ] && exit 0

url="$(printf '%s' "$release" \
  | sed -n 's/.*"browser_download_url": *"\([^"]*linux-x86_64\.tar\.gz\)".*/\1/p' \
  | head -1)"
[ -n "$url" ] || die "no linux tarball in $tag"

log "deploying $tag"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT INT TERM

curl -fL -o "$TMP/kotodex.tar.gz" "$url" || die "download failed"
# The checksum is published beside the tarball. A truncated download that still
# unpacked would leave the demo serving a broken binary until the next release.
if curl -fsSL -o "$TMP/sum" "$url.sha256" 2>/dev/null; then
  expected="$(cut -d' ' -f1 <"$TMP/sum")"
  actual="$(sha256sum "$TMP/kotodex.tar.gz" | cut -d' ' -f1)"
  [ "$expected" = "$actual" ] || die "checksum mismatch on $tag"
fi

# Moved into place only once it is whole, so a failed download leaves the
# running demo alone.
mv "$TMP/kotodex.tar.gz" "$TARBALL"

cd "$STACK"
docker compose build || die "compose build failed"
docker compose up -d || die "compose up failed"

# Written last: a failed deploy has to be retried on the next run.
printf '%s\n' "$tag" >"$STAMP"
log "deployed $tag"
