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
    pyside6:pacman) echo pyside6 ;;
    pyside6:apt) echo python3-pyside6.qtwebenginequick ;;
    pyside6:dnf) echo python3-pyside6 ;;
    pyside6:zypper) echo python3-pyside6 ;;

    qt6-webengine:pacman) echo qt6-webengine ;;
    qt6-webengine:apt) echo libqt6webenginecore6 ;;
    qt6-webengine:dnf) echo qt6-qtwebengine ;;
    qt6-webengine:zypper) echo qt6-webengine ;;

    # What a pip PySide6 needs from the system: the wheel carries Qt but not
    # the libraries under it. The distribution's own Qt WebEngine pulls that
    # whole set as its dependencies, which is why it is named here rather than
    # a list of lib packages that would drift.
    webengine-runtime:apt) echo "libqt6webenginecore6 libasound2t64" ;;
    webengine-runtime:dnf) echo "qt6-qtwebengine alsa-lib" ;;
    webengine-runtime:zypper) echo "qt6-webengine libasound2" ;;
    webengine-runtime:pacman) echo "qt6-webengine alsa-lib" ;;

    layer-shell-qt:pacman) echo layer-shell-qt ;;
    layer-shell-qt:apt) echo qt6-wayland ;;
    layer-shell-qt:dnf) echo layer-shell-qt ;;
    layer-shell-qt:zypper) echo layer-shell-qt6 ;;

    pactl:pacman) echo libpulse ;;
    pactl:apt) echo pulseaudio-utils ;;
    pactl:dnf) echo pulseaudio-utils ;;
    pactl:zypper) echo pulseaudio-utils ;;
    pactl:apk) echo pulseaudio-utils ;;

    # Fedora's own build; plain `ffmpeg` is RPM Fusion, which a fresh install
    # does not have enabled.
    ffmpeg:dnf) echo ffmpeg-free ;;

    python:pacman) echo python ;;
    python:*) echo python3 ;;

    # Only Debian and Ubuntu ship venv separately; everywhere else it comes
    # with the interpreter.
    python3-venv:apt) echo python3-venv ;;
    python3-venv:pacman) echo python ;;
    python3-venv:*) echo python3 ;;

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

# --------------------------------------------------------------- python --

# Where a pip PySide6 goes when the distribution does not package one.
# `--system-site-packages`, so everything else python needs here still comes
# from the distribution and only PySide6 is pip's.
KOTODEX_VENV="${KOTODEX_VENV:-$HOME/.local/share/kotodex/venv}"

# **The** interpreter every Python part of Kotodex runs under: the launcher, the
# overlay, the line logger and the VAD. One answer, because a second one drifts
# from it exactly when it matters — a doctor that probes a different interpreter
# from the one the overlay will use reports on a machine nobody is running.
#
# The venv `setup.sh` builds, which is where `requirements.txt` lands and, on a
# distribution that packages no PySide6, where pip's goes too. Built with
# `--system-site-packages`, so it can import everything the system interpreter
# can *plus* its own — which is why there is no import check here and no
# fallback worth having: anything the venv cannot import, python3 cannot either.
# The bare name is only for a machine setup.sh has never been run on.
# Always an absolute path: callers test the answer with `-x`, which a bare name
# fails whatever is on PATH.
kotodex_python() {
  if [ -x "$KOTODEX_VENV/bin/python" ]; then
    echo "$KOTODEX_VENV/bin/python"
  else
    command -v python3 || echo /usr/bin/python3
  fi
}

# `pip` or `system` — where the PySide6 that will be imported actually lives.
# The venv is built with `--system-site-packages` and always exists, so it can
# import the distribution's PySide6; that it resolves through the venv says
# nothing about who installed it. Asking the module for its own path does.
pyside6_source() {
  local py file
  py="$(kotodex_python)"
  # QtWebEngineQuick, not PySide6 alone: the base package without the WebEngine
  # addon cannot draw the overlay, and reporting it as present hides that.
  file="$("$py" -c "import PySide6.QtWebEngineQuick, PySide6, sys; sys.stdout.write(PySide6.__file__)" 2>/dev/null)" \
    || return 1
  case "$file" in
    "$KOTODEX_VENV"/*) echo pip ;;
    *) echo system ;;
  esac
}

# Whether this distribution packages PySide6 at all. Ubuntu 24.04 LTS and
# Debian 12 do not — they carry PySide2 only — so the pip venv is the only way
# to run there, and offering it needs to be a decision made from a fact.
distro_packages_pyside6() {
  local mgr name
  mgr="$(pkg_manager || echo unknown)"
  name="$(pkg_name "$mgr" pyside6)"
  case "$mgr" in
    apt) apt-cache show "$name" >/dev/null 2>&1 ;;
    dnf) dnf -q info "$name" >/dev/null 2>&1 ;;
    zypper) zypper -q info "$name" >/dev/null 2>&1 ;;
    pacman) pacman -Si "$name" >/dev/null 2>&1 ;;
    # Unknown package manager: assume it does, so nothing is pushed at pip on a
    # distribution nobody has checked.
    *) return 0 ;;
  esac
}

# Whether the ffmpeg on PATH can record from PulseAudio. Installed is not
# enough: the ring buffer reads the speaker's monitor through `-f pulse`, and an
# ffmpeg built without it — Fedora's own package, and the static builds people
# put in ~/bin for other tools — dies on the first frame.
#
# The demuxer list, not `-devices`: a build with pulse output only appears there
# and still cannot record.
#
# Read into a variable rather than piped into `grep -q`: under `pipefail` the
# grep exits at the first match, ffmpeg dies of SIGPIPE, and the pipeline reports
# 141 — so a working ffmpeg reads as a broken one.
ffmpeg_records_pulse() {
  local demuxers
  demuxers="$(ffmpeg -hide_banner -demuxers 2>/dev/null)" || return 1
  [[ $demuxers =~ (^|[[:space:]])pulse([[:space:]]|$) ]]
}
