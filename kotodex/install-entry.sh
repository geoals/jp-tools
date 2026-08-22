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
ICONS="$HOME/.local/share/icons/hicolor"

if [ "${1:-}" = "--uninstall" ]; then
  rm -f "$BIN/kotodex" "$BIN/kotodex-capture" "$APPS/kotodex.desktop"
  for size in 48 64 128 256 512; do
    rm -f "$ICONS/${size}x${size}/apps/kotodex.png"
  done
  rm -f "$ICONS/scalable/apps/kotodex.svg"
  command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" || true
  echo "removed. Your databases under ~/.local/share/jp-tools were not touched."
  exit 0
fi

mkdir -p "$BIN" "$APPS" "$ICONS/scalable/apps"
ln -sf "$HERE/kotodex" "$BIN/kotodex"
ln -sf "$REPO/vn-mine/kotodex-capture" "$BIN/kotodex-capture"

for size in 48 64 128 256 512; do
  mkdir -p "$ICONS/${size}x${size}/apps"
  cp -f "$HERE/icons/kotodex-$size.png" "$ICONS/${size}x${size}/apps/kotodex.png"
done
cp -f "$HERE/kotodex.svg" "$ICONS/scalable/apps/kotodex.svg"
cp -f "$HERE/kotodex.desktop" "$APPS/kotodex.desktop"

command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -qtf "$ICONS" || true

echo "installed: $APPS/kotodex.desktop"
case ":$PATH:" in
  *":$BIN:"*) ;;
  *) echo "note: $BIN is not on PATH. Add it:"
     echo "  fish_add_path $BIN      # fish"
     echo "  export PATH=\"$BIN:\$PATH\"  # bash/zsh" ;;
esac
