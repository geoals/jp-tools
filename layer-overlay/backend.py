"""Which mechanism puts the surface above a fullscreen window.

Two exist, and which is available is a property of the compositor rather than
of this program:

- **layer-shell** — `zwlr_layer_shell_v1`. A surface on the overlay layer is
  above everything by protocol, and the compositor takes the input region from
  `wl_surface.set_input_region`. KDE and wlroots offer it; GNOME does not.
- **x11** — `_NET_WM_STATE_ABOVE` on an XWayland window, with the input region
  set through XShape. Works wherever XWayland does, which includes GNOME.

The choice has to be made **before `QGuiApplication` exists**: it decides the Qt
platform plugin, and that is read once when the application is constructed. So
this is a probe of the environment, not of a running Qt.
"""

import os
import shutil
import subprocess

LAYER_SHELL = "layer-shell"
X11 = "x11"

#: The interface whose presence *is* the layer-shell backend.
PROTOCOL = "zwlr_layer_shell_v1"


def _advertises_layer_shell() -> bool:
    """Ask the compositor what it offers.

    `wayland-info` is the only answer that is not a guess about a desktop's
    name. It is a small, widely packaged tool, and its absence is not a failure
    — it just means falling back to the guess below.
    """
    if not shutil.which("wayland-info"):
        return False
    try:
        out = subprocess.run(
            ["wayland-info"], capture_output=True, text=True, timeout=5
        )
    except (OSError, subprocess.SubprocessError):
        return False
    return PROTOCOL in out.stdout


def _qml_module_present() -> bool:
    """layer-shell also needs the Qt binding, which is a separate package.

    A compositor that offers the protocol with no `org.kde.layershell` to drive
    it would load the QML and fail — after the platform plugin is already
    chosen and too late to change.
    """
    roots = os.environ.get("QML2_IMPORT_PATH", "").split(":")
    roots += ["/usr/lib/qt6/qml", "/usr/lib64/qt6/qml", "/usr/lib/x86_64-linux-gnu/qt6/qml"]
    return any(
        root and os.path.isdir(os.path.join(root, "org", "kde", "layershell"))
        for root in roots
    )


def choose() -> tuple[str, str]:
    """The backend to use and one line saying why, for the log and the doctor."""
    forced = os.environ.get("LAYER_OVERLAY_BACKEND", "").strip().lower()
    if forced in (LAYER_SHELL, X11):
        return forced, "set by LAYER_OVERLAY_BACKEND"

    if not os.environ.get("WAYLAND_DISPLAY"):
        return X11, "no Wayland session"
    if not _qml_module_present():
        return X11, "org.kde.layershell is not installed"
    if _advertises_layer_shell():
        return LAYER_SHELL, f"the compositor offers {PROTOCOL}"
    if shutil.which("wayland-info"):
        return X11, f"the compositor does not offer {PROTOCOL}"

    # No way to ask, so name the one compositor family known not to have it
    # rather than pick the backend that fails silently.
    desktop = os.environ.get("XDG_CURRENT_DESKTOP", "")
    if "GNOME" in desktop.upper():
        return X11, "GNOME, and wayland-info is not installed to confirm"
    return LAYER_SHELL, "assumed; install wayland-info to check"


def apply_environment(backend: str) -> None:
    """Set what Qt reads at construction time. Must run before QGuiApplication.

    On a native Wayland surface Qt accepts `WindowStaysOnTopHint` and silently
    does nothing with it, so the X11 backend has to ask for the xcb plugin
    rather than inherit the session's default.
    """
    if backend == LAYER_SHELL:
        os.environ.setdefault("QT_WAYLAND_SHELL_INTEGRATION", "layer-shell")
        os.environ.setdefault("QT_QPA_PLATFORM", "wayland")
    else:
        os.environ["QT_QPA_PLATFORM"] = "xcb"
        os.environ.pop("QT_WAYLAND_SHELL_INTEGRATION", None)


if __name__ == "__main__":
    name, why = choose()
    print(f"{name}\t{why}")
