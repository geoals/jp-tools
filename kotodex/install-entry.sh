#!/usr/bin/env bash
# Put Kotodex in the application menu: the binaries on PATH, the icon in
# hicolor, the desktop entry where the launcher looks for it.
#
#   kotodex/install-entry.sh [--uninstall]
#
# Everything goes under ~/.local, so it needs no root and removing it takes
# nothing else with it.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
BIN="$HOME/.local/bin"
APPS="$HOME/.local/share/applications"
# Reverse-DNS off kotodex.com: what Flatpak and the desktop spec want an app
# called, and what the launcher passes to setDesktopFileName.
APP_ID="com.kotodex.Kotodex"
ICONS="$HOME/.local/share/icons/hicolor"

if [ "${1:-}" = "--uninstall" ]; then
  rm -f "$BIN/kotodex" "$BIN/kotodex-capture" "$APPS/$APP_ID.desktop" "$APPS/kotodex.desktop"
  for size in 48 64 128 256 512; do
    rm -f "$ICONS/${size}x${size}/apps/$APP_ID.png" "$ICONS/${size}x${size}/apps/kotodex.png"
  done
  rm -f "$ICONS/scalable/apps/$APP_ID.svg" "$ICONS/scalable/apps/kotodex.svg"
  command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" || true
  echo "removed. Your databases under ~/.local/share/kotodex were not touched."
  exit 0
fi

mkdir -p "$BIN" "$APPS" "$ICONS/scalable/apps"
ln -sf "$HERE/kotodex" "$BIN/kotodex"
ln -sf "$REPO/vn-mine/kotodex-capture" "$BIN/kotodex-capture"

for size in 48 64 128 256 512; do
  mkdir -p "$ICONS/${size}x${size}/apps"
  cp -f "$HERE/icons/kotodex-$size.png" "$ICONS/${size}x${size}/apps/$APP_ID.png"
done
cp -f "$HERE/kotodex.svg" "$ICONS/scalable/apps/$APP_ID.svg"
# An install from before the app id was reverse-DNS leaves a second entry and a
# second icon, both of which show up in the menu beside the real one.
rm -f "$APPS/kotodex.desktop"
for size in 48 64 128 256 512; do
  rm -f "$ICONS/${size}x${size}/apps/kotodex.png"
done
rm -f "$ICONS/scalable/apps/kotodex.svg"
# Exec is rewritten to the absolute path rather than left as the bare name. A
# desktop entry is launched by the session's own systemd/DE, whose PATH is not
# the shell's — on a distribution that does not add ~/.local/bin there, a bare
# name is an entry that does nothing at all when clicked, with nowhere to see
# why. The `|` delimiter because the path contains slashes.
sed "s|^Exec=kotodex$|Exec=$BIN/kotodex|" "$HERE/$APP_ID.desktop" >"$APPS/$APP_ID.desktop"

command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -qtf "$ICONS" || true

echo "installed: $APPS/$APP_ID.desktop"
case ":$PATH:" in
  *":$BIN:"*) ;;
  *) echo "note: $BIN is not on PATH. Add it:"
     echo "  fish_add_path $BIN      # fish"
     echo "  export PATH=\"$BIN:\$PATH\"  # bash/zsh" ;;
esac
