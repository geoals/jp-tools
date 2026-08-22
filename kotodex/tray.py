"""The tray icon, and what to do when there is no tray.

GNOME ships no tray without the AppIndicator extension, so a launcher that
minimises into one would vanish. When the system says there is no tray, say so
once and keep the overlay on screen instead of hiding it.
"""

import subprocess
import webbrowser
from pathlib import Path

from PySide6.QtGui import QAction, QIcon
from PySide6.QtWidgets import QMenu, QSystemTrayIcon

REPO = Path(__file__).resolve().parent.parent
OVERLAY = REPO / "read-stats" / "overlay" / "vn-overlay.sh"
ICON = REPO / "kotodex" / "kotodex.svg"


class Tray:
    def __init__(self, app, children, read_stats_url, log):
        self.app = app
        self.children = children
        self.url = read_stats_url
        self.log = log
        self.available = QSystemTrayIcon.isSystemTrayAvailable()
        self.icon = None
        if not self.available:
            log("no system tray here — the overlay stays on screen; close it to quit")
            return

        self.icon = QSystemTrayIcon(QIcon(str(ICON)), app)
        self.icon.setToolTip(self._tooltip())
        menu = QMenu()
        for label, slot in (
            ("Show overlay", self.show_overlay),
            ("Hide overlay", self.hide_overlay),
            ("Open reading stats", self.open_stats),
            ("Pause capture", self.pause_capture),
            ("Doctor", self.doctor),
            ("Quit", self.app.quit),
        ):
            action = QAction(label, menu)
            action.triggered.connect(slot)
            menu.addAction(action)
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
        subprocess.Popen([str(OVERLAY), "start"], cwd=REPO)

    def hide_overlay(self):
        subprocess.run([str(OVERLAY), "stop"], cwd=REPO, capture_output=True)

    def open_stats(self):
        webbrowser.open(self.url)

    def pause_capture(self):
        subprocess.run(
            ["curl", "-s", "-X", "POST", f"{self.url}/api/capture/pause"],
            capture_output=True,
        )

    def doctor(self):
        script = REPO / "scripts" / "kotodex-doctor.sh"
        for term in ("konsole", "gnome-terminal", "xterm"):
            if subprocess.run(["which", term], capture_output=True).returncode == 0:
                subprocess.Popen([term, "-e", "bash", "-c", f"{script}; read -r"])
                return
        self.log("no terminal to show the doctor in; run scripts/kotodex-doctor.sh")
