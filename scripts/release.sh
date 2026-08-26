#!/usr/bin/env bash
# Cut a release: tag it, wait for the build, publish it.
#
#   scripts/release.sh <version> [--yes] [--dry-run]
#
# The version has no leading v — `scripts/release.sh 0.1.1` makes tag v0.1.1
# and tarball kotodex-0.1.1-linux-x86_64.tar.gz.
#
# The workflow builds a draft release so the notes and the assets can be looked
# at before anyone can download them. This keeps that gate: it shows what was
# built and asks before publishing. --yes takes the default without asking.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

bold=$'\033[1m'; red=$'\033[31m'; green=$'\033[32m'; off=$'\033[0m'
[ -t 1 ] || { bold=""; red=""; green=""; off=""; }
say() { printf '%s==>%s %s\n' "$bold" "$off" "$1"; }
die() { printf '%s✗%s %s\n' "$red" "$off" "$1" >&2; exit 1; }

VERSION=""
ASSUME_YES=0
DRY_RUN=0
while [ $# -gt 0 ]; do
  case "$1" in
    --yes|-y) ASSUME_YES=1 ;;
    --dry-run|-n) DRY_RUN=1 ;;
    --help|-h) sed -n '2,${/^#/!q; s/^# \?//p;}' "${BASH_SOURCE[0]}"; exit 0 ;;
    -*) die "unknown option: $1" ;;
    *) [ -z "$VERSION" ] || die "one version, not two"; VERSION="$1" ;;
  esac
  shift
done

[ -n "$VERSION" ] || die "which version? scripts/release.sh 0.1.1"
case "$VERSION" in
  v*) die "no leading v — the tag gets one, the tarball does not" ;;
  *[!0-9.]*|*..*|.*|*.) die "not a version number: $VERSION" ;;
esac
TAG="v$VERSION"

for bin in git gh; do
  command -v "$bin" >/dev/null || die "$bin is required"
done

# Everything that would make the tag point at something other than what is
# being reviewed, checked before anything is pushed.
say "checking the tree"
[ -z "$(git status --porcelain)" ] || die "uncommitted changes — commit or stash them first"
branch="$(git rev-parse --abbrev-ref HEAD)"
[ "$branch" = master ] || die "on $branch, not master"
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null && die "$TAG already exists"
git fetch -q origin
behind="$(git rev-list --count "HEAD..origin/master")"
[ "$behind" = 0 ] || die "origin/master is $behind commits ahead — pull first"

# The workflow prepends this file to the notes GitHub generates from the
# commits since the last tag, so what is new in a release is never something to
# remember to write. The file itself says what Kotodex is and how to install it,
# which does not change between releases.
notes="docs/release-notes.md"
[ -r "$notes" ] || die "$notes is missing — the workflow prepends it to the release notes"

if [ "$DRY_RUN" = 1 ]; then
  say "would tag $TAG and push it, then publish the draft"
  exit 0
fi

if [ "$ASSUME_YES" != 1 ]; then
  read -r -p "    release $TAG? [Y/n] " answer </dev/tty || exit 1
  case "$answer" in [nN]*) exit 1 ;; esac
fi

say "pushing master"
git push -q origin master

say "tagging $TAG"
git tag "$TAG"
git push -q origin "$TAG"

# The tag push starts the workflow, which needs a moment to exist before it can
# be watched.
say "waiting for the build"
run=""
for _ in $(seq 30); do
  run="$(gh run list --workflow release.yml --branch "$TAG" --limit 1 --json databaseId --jq '.[0].databaseId' 2>/dev/null || true)"
  [ -n "$run" ] && break
  sleep 2
done
[ -n "$run" ] || die "the workflow did not start — see gh run list"
gh run watch "$run" --exit-status >/dev/null || die "the build failed — gh run view $run --log-failed"

say "built"
gh release view "$TAG" --json assets --jq '.assets[] | "    \(.name)  \(.size) bytes"'

if [ "$ASSUME_YES" != 1 ]; then
  read -r -p "    publish $TAG? [Y/n] " answer </dev/tty || exit 1
  case "$answer" in [nN]*) say "left as a draft — gh release edit $TAG --draft=false"; exit 0 ;; esac
fi

gh release edit "$TAG" --draft=false >/dev/null
printf '%s✓%s %s\n' "$green" "$off" "$(gh release view "$TAG" --json url --jq .url)"
