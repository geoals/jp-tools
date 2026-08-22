#!/usr/bin/env bash
# Checks `platform.sh` against a faked /etc/os-release for each distro family.
set -uo pipefail
cd "$(dirname "$(readlink -f "$0")")"
source ./platform.sh

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
fail=0

check() {
  local name="$1" want="$2" got="$3"
  if [ "$got" = "$want" ]; then
    echo "ok    $name"
  else
    echo "FAIL  $name: want '$want', got '$got'"
    fail=1
  fi
}

manager_for() {
  printf '%s\n' "$1" >"$tmp/os-release"
  PLATFORM_OS_RELEASE="$tmp/os-release" pkg_manager
}

check arch      pacman "$(manager_for 'ID=arch')"
check ubuntu    apt    "$(manager_for 'ID=ubuntu
ID_LIKE=debian')"
check mint      apt    "$(manager_for 'ID=linuxmint
ID_LIKE="ubuntu debian"')"
check fedora    dnf    "$(manager_for 'ID=fedora')"
check rocky     dnf    "$(manager_for 'ID=rocky
ID_LIKE="rhel centos fedora"')"
check opensuse  zypper "$(manager_for 'ID="opensuse-tumbleweed"
ID_LIKE="opensuse suse"')"
check alpine    apk    "$(manager_for 'ID=alpine')"
check void      xbps   "$(manager_for 'ID=void')"
check nixos     nix    "$(manager_for 'ID=nixos')"

# The names that actually differ per distro are the point of the table.
check "pyside6 on apt"  python3-pyside6.qtwebengine "$(pkg_name apt pyside6)"
check "pactl on pacman" libpulse                    "$(pkg_name pacman pactl)"
check "ffmpeg anywhere" ffmpeg                      "$(pkg_name dnf ffmpeg)"

exit "$fail"
