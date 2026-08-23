#!/usr/bin/env bash
# Which distribution this is, and what to type to install something on it.
#
# Sourced by setup.sh and the doctor so a missing dependency is reported with a
# command that works here rather than a package name to go and look up.
#
#   source scripts/lib/platform.sh
#   pkg_manager                  # pacman | apt | dnf | zypper | apk | xbps | nix
#   pkg_install_cmd ffmpeg jq    # the whole line, ready to paste
#
# Package names differ per distro for most of what we need, so they are a table
# rather than a guess: `pyside6` is `python-pyside6` on Arch and
# `python3-pyside6.qtwebengine` on Debian.

# The `ID` and `ID_LIKE` of the running system. `PLATFORM_OS_RELEASE` overrides
# the file, which is what the tests point somewhere else.
_os_release_field() {
  local file="${PLATFORM_OS_RELEASE:-/etc/os-release}" key="$1" line value
  [ -r "$file" ] || return 1
  while IFS= read -r line; do
    case "$line" in
      "$key"=*)
        value="${line#*=}"
        value="${value%\"}"
        value="${value#\"}"
        printf '%s\n' "$value"
        return 0
        ;;
    esac
  done <"$file"
  return 1
}

# The package manager, from `ID` first and then `ID_LIKE` — a derivative names
# its parent there, so Linux Mint reaches apt without being listed.
pkg_manager() {
  local id ids
  id="$(_os_release_field ID || true)"
  ids="$id $(_os_release_field ID_LIKE || true)"
  for id in $ids; do
    case "$id" in
      arch|archlinux|manjaro|endeavouros) echo pacman; return 0 ;;
      debian|ubuntu|linuxmint|pop) echo apt; return 0 ;;
      fedora|rhel|centos|rocky|almalinux) echo dnf; return 0 ;;
      opensuse*|suse|sles) echo zypper; return 0 ;;
      alpine) echo apk; return 0 ;;
      void) echo xbps; return 0 ;;
      nixos) echo nix; return 0 ;;
    esac
  done
  # Nothing recognised: fall back to whatever is actually on PATH, which covers
  # a distro not in the list above.
  local candidate
  for candidate in pacman apt dnf zypper apk xbps-install nix-env; do
    if command -v "$candidate" >/dev/null 2>&1; then
      [ "$candidate" = xbps-install ] && candidate=xbps
      [ "$candidate" = nix-env ] && candidate=nix
      echo "$candidate"
      return 0
    fi
  done
  return 1
}

# The distro's name for one of ours. Unknown generic names pass through
# unchanged, which is right for the many that are spelt the same everywhere.
pkg_name() {
  local mgr="$1" generic="$2"
  case "$generic:$mgr" in
    pyside6:pacman) echo python-pyside6 ;;
    pyside6:apt) echo python3-pyside6.qtwebenginequick ;;
    pyside6:dnf) echo python3-pyside6 ;;
    pyside6:zypper) echo python3-pyside6 ;;

    qt6-webengine:pacman) echo qt6-webengine ;;
    qt6-webengine:apt) echo libqt6webenginecore6 ;;
    qt6-webengine:dnf) echo qt6-qtwebengine ;;
    qt6-webengine:zypper) echo qt6-webengine ;;

    layer-shell-qt:pacman) echo layer-shell-qt ;;
    layer-shell-qt:apt) echo qt6-wayland ;;
    layer-shell-qt:dnf) echo layer-shell-qt ;;
    layer-shell-qt:zypper) echo layer-shell-qt6 ;;

    pactl:pacman) echo libpulse ;;
    pactl:apt) echo pulseaudio-utils ;;
    pactl:dnf) echo pulseaudio-utils ;;
    pactl:zypper) echo pulseaudio-utils ;;
    pactl:apk) echo pulseaudio-utils ;;

    python:pacman) echo python ;;
    python:*) echo python3 ;;

    import:pacman) echo imagemagick ;;
    import:apt) echo imagemagick ;;
    import:*) echo ImageMagick ;;

    screenshot:pacman) echo spectacle ;;
    screenshot:apt) echo grim ;;
    screenshot:dnf) echo grim ;;
    screenshot:zypper) echo grim ;;

    *) echo "$generic" ;;
  esac
}

# The whole install line for one or more generic names.
pkg_install_cmd() {
  local mgr names=() generic
  mgr="$(pkg_manager || echo unknown)"
  for generic in "$@"; do names+=("$(pkg_name "$mgr" "$generic")"); done
  case "$mgr" in
    pacman) echo "sudo pacman -S ${names[*]}" ;;
    apt) echo "sudo apt install ${names[*]}" ;;
    dnf) echo "sudo dnf install ${names[*]}" ;;
    zypper) echo "sudo zypper install ${names[*]}" ;;
    apk) echo "sudo apk add ${names[*]}" ;;
    xbps) echo "sudo xbps-install -S ${names[*]}" ;;
    nix) echo "nix-env -iA ${names[*]/#/nixpkgs.}" ;;
    *) echo "install: ${names[*]}" ;;
  esac
}
