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

# This directory is a copy of demo/ in the repository, and for a long time only
# the tarball was refreshed in it. A release that changed the entrypoint or the
# compose file left the demo running against the old ones until it broke.
#
# Taken at the deployed tag, so they match the binary rather than whatever
# master happens to say. Anything edited here by hand is overwritten - the
# repository is where a change to them belongs.
for f in Dockerfile compose.yaml entrypoint.sh; do
  curl -fsSL -o "$TMP/$f" \
    "https://raw.githubusercontent.com/$REPO/$tag/demo/$f" \
    || die "could not fetch demo/$f at $tag"
done
# Moved only once all three are down, so a failure part way through does not
# leave the stack half updated.
for f in Dockerfile compose.yaml entrypoint.sh; do
  mv "$TMP/$f" "$STACK/$f"
done

cd "$STACK"
docker compose build || die "compose build failed"
docker compose up -d || die "compose up failed"

# Written last: a failed deploy has to be retried on the next run.
printf '%s\n' "$tag" >"$STAMP"
log "deployed $tag"
