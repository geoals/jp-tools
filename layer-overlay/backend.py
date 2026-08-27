"""Which mechanism puts the surface above a fullscreen window.

Three exist, and which is available is a property of the display server rather
than of this program:

- **layer-shell** — `zwlr_layer_shell_v1`. A surface on the overlay layer is
  above everything by protocol, and the compositor takes the input region from
  `wl_surface.set_input_region`. KDE and wlroots offer it; GNOME does not.
- **x11** — `_NET_WM_STATE_ABOVE` on an XWayland window, with the input region
  set through XShape. Works wherever XWayland does, which includes GNOME.
- **windows** — a layered topmost window. Windows has no input region at all,
  so [`wininput`] gets the same effect by toggling `WS_EX_TRANSPARENT` as the
  cursor crosses into what the page has drawn.

The choice has to be made **before `QGuiApplication` exists**: it decides the Qt
platform plugin, and that is read once when the application is constructed. So
this is a probe of the environment, not of a running Qt.
"""

import os
import shutil
import subprocess
import sys

LAYER_SHELL = "layer-shell"
X11 = "x11"
WINDOWS = "windows"

#: The interface whose presence *is* the layer-shell backend.
PROTOCOL = "zwlr_layer_shell_v1"

#: What makes QtWebEngine keep the page's alpha under X11. See
#: [`apply_environment`].
TRANSPARENT_VISUALS = "--enable-transparent-visuals"


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
    roots += _qt_qml_paths()
    roots += ["/usr/lib/qt6/qml", "/usr/lib64/qt6/qml", "/usr/lib/x86_64-linux-gnu/qt6/qml"]
    return any(
        root and os.path.isdir(os.path.join(root, "org", "kde", "layershell"))
        for root in roots
    )


def _qt_qml_paths() -> list[str]:
    """Where *this* interpreter's Qt looks for QML modules.

    The distribution's directory is the wrong answer for a pip PySide6: it
    ships its own Qt, which does not read /usr/lib/qt6/qml, so a system
    layer-shell-qt beside it is not loadable. Asking Qt itself is the only way
    to tell the two apart.
    """
    try:
        from PySide6.QtCore import QLibraryInfo
    except ImportError:
        return []
    return [QLibraryInfo.path(QLibraryInfo.LibraryPath.QmlImportsPath)]


def choose() -> tuple[str, str]:
    """The backend to use and one line saying why, for the log and the doctor."""
    forced = os.environ.get("LAYER_OVERLAY_BACKEND", "").strip().lower()
    if forced in (LAYER_SHELL, X11, WINDOWS):
        return forced, "set by LAYER_OVERLAY_BACKEND"

    # Not a probe: there is one way to put a window above a fullscreen one here,
    # and nothing to fall back to if it does not work.
    if sys.platform == "win32":
        return WINDOWS, "Windows"

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

    The X11 backend also has to ask Chromium for a transparent visual. A
    Wayland surface is transparent by protocol, but on X11 QtWebEngine clears
    every frame opaque whatever `backgroundColor` the view is given — so the
    page's own translucent backdrop arrives as solid black, and an overlay that
    is supposed to show the window underneath instead hides it completely.
    """
    if backend == WINDOWS:
        # Nothing: there is one platform plugin, it is the default, and a
        # layered window keeps the page's alpha without asking Chromium for a
        # visual.
        return
    if backend == LAYER_SHELL:
        os.environ.setdefault("QT_WAYLAND_SHELL_INTEGRATION", "layer-shell")
        os.environ.setdefault("QT_QPA_PLATFORM", "wayland")
    else:
        os.environ["QT_QPA_PLATFORM"] = "xcb"
        os.environ.pop("QT_WAYLAND_SHELL_INTEGRATION", None)
        flags = os.environ.get("QTWEBENGINE_CHROMIUM_FLAGS", "")
        if TRANSPARENT_VISUALS not in flags:
            os.environ["QTWEBENGINE_CHROMIUM_FLAGS"] = (
                f"{flags} {TRANSPARENT_VISUALS}".strip()
            )


if __name__ == "__main__":
    name, why = choose()
    print(f"{name}\t{why}")
