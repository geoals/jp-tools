"""The tray icon, and what to do when there is no tray.

GNOME ships no tray without the AppIndicator extension, so a launcher that
minimises into one would vanish. When the system says there is no tray, say so
once and keep the overlay on screen instead of hiding it.
"""

import json
import shutil
import subprocess
import urllib.error
import urllib.request
import webbrowser

from PySide6.QtGui import QAction, QIcon
from PySide6.QtWidgets import QMenu, QSystemTrayIcon

# From the launcher, not rebuilt here: "where is the overlay script" is one
# answer, and a tray that reached for its own copy is the one that goes stale.
from kotodex import DOCTOR_SH, ICON, OVERLAY_SH, REPO


class Tray:
    def __init__(self, app, children, kotodex_server_url, log):
        self.app = app
        self.children = children
        self.url = kotodex_server_url
        self.log = log
        self.available = QSystemTrayIcon.isSystemTrayAvailable()
        # Set by kotodex.main: the restart has to happen in the process that
        # supervises the children, not beside it.
        self.restart_here = None
        self.icon = None
        if not self.available:
            log("no system tray here — the overlay stays on screen; Ctrl-C to quit")
            return

        self.icon = QSystemTrayIcon(QIcon(str(ICON)), app)
        self.icon.setToolTip(self._tooltip())
        menu = QMenu()
        self.pause_action = None
        for label, slot in (
            ("Show overlay", self.show_overlay),
            ("Hide overlay", self.hide_overlay),
            ("Open reading stats", self.open_stats),
            ("Pause capture", self.toggle_capture),
            ("Restart everything", self.restart),
            ("Doctor", self.doctor),
            ("Quit", self.app.quit),
        ):
            action = QAction(label, menu)
            action.triggered.connect(slot)
            menu.addAction(action)
            if slot == self.toggle_capture:
                self.pause_action = action
        # The overlay has the same toggle, so the label is refreshed on open
        # rather than only after this menu was the one that flipped it.
        menu.aboutToShow.connect(self.refresh_pause_label)
        self.icon.setContextMenu(menu)
        self.icon.activated.connect(
            lambda reason: self.show_overlay()
            if reason == QSystemTrayIcon.Trigger
            else None
        )
        self.icon.show()

    def _tooltip(self):
        adopted = [c.name for c in self.children if c.adopted]
        if not adopted:
            return "Kotodex"
        # Quitting leaves these behind on purpose, and the tooltip is where
        # that is visible before it happens rather than after.
        return "Kotodex — already running, left alone on quit: " + ", ".join(adopted)

    def show_overlay(self):
        # `ensure`, not `start`: this is also where a second launch of the
        # desktop entry lands, and restarting a running overlay would throw
        # away the page the reader is looking at.
        subprocess.Popen([OVERLAY_SH, "ensure"], cwd=REPO)

    def hide_overlay(self):
        subprocess.run([OVERLAY_SH, "stop"], cwd=REPO, capture_output=True)

    def open_stats(self):
        webbrowser.open(self.url)

    def _api(self, path, method="GET"):
        req = urllib.request.Request(f"{self.url}{path}", method=method)
        try:
            with urllib.request.urlopen(req, timeout=2) as r:
                return json.load(r)
        except (urllib.error.URLError, OSError, ValueError):
            return None

    def toggle_capture(self):
        result = self._api("/api/capture/pause", method="POST")
        if result is None:
            self.log("capture: kotodex-server did not answer the pause toggle")
            return
        self._set_pause_label(result.get("paused", False))

    def refresh_pause_label(self):
        settings = self._api("/api/settings")
        if settings is not None:
            self._set_pause_label(settings.get("capture_paused", False))

    def _set_pause_label(self, paused: bool):
        if self.pause_action is not None:
            self.pause_action.setText("Resume capture" if paused else "Pause capture")

    def restart(self):
        """Pick up new code, whoever started each piece.

        Adopting is the launcher's whole design and the reason this cannot be
        done by quitting and reopening: a component it adopted is one it never
        touches, and the capture daemon is usually systemd's.

        Done in this process, not as a child: it is the one that supervises
        kotodex-server, and the gap where the port is closed must not read as a
        crash. `kotodex.main` sets the callback.
        """
        if self.restart_here is None:
            self.log("nothing wired up to restart with")
            return
        self.restart_here()

    def doctor(self):
        # `--` for the terminals that want it and `-e` for the one that does not:
        # gnome-terminal dropped `-e` and konsole never took `--`, so each gets
        # the form it accepts rather than one form that half of them refuse.
        for term, flag in (
            ("konsole", "-e"),
            ("gnome-terminal", "--"),
            ("xfce4-terminal", "-x"),
            ("xterm", "-e"),
        ):
            if shutil.which(term) is None:
                continue
            subprocess.Popen([term, flag, "bash", "-c", f"{DOCTOR_SH}; read -r"])
            return
        self.log("no terminal to show the doctor in; run scripts/kotodex-doctor.sh")
