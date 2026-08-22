#!/usr/bin/env python3
"""T0.4: does a plain always-on-top X11 window stack above a fullscreen game,
and do clicks fall through where it has not drawn?

Run it beside a fullscreen test target (`mpv --gpu-context=x11 --fs <file>`,
`glxgears -fullscreen`). Red box visible over the fullscreen window = stacking
works. Clicking the transparent margin should reach the window underneath.
"""

import sys

from PySide6.QtCore import Qt, QRect
from PySide6.QtGui import QRegion, QPainter, QColor
from PySide6.QtWidgets import QApplication, QWidget

BOX = QRect(20, 20, 160, 60)


class Probe(QWidget):
    def __init__(self):
        super().__init__()
        self.setWindowFlags(
            Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint | Qt.Tool
        )
        self.setAttribute(Qt.WA_TranslucentBackground)
        self.setGeometry(100, 100, 200, 100)
        # The input region: clicks land only on the drawn box, everything else
        # falls through to whatever is underneath.
        self.setMask(QRegion(BOX))

    def paintEvent(self, _):
        p = QPainter(self)
        p.fillRect(BOX, QColor(220, 40, 40, 220))
        p.setPen(Qt.white)
        p.drawText(BOX, Qt.AlignCenter, "on top?")

    def mousePressEvent(self, _):
        print("click landed on the overlay")


app = QApplication(sys.argv)
w = Probe()
w.show()
print(f"platform: {app.platformName()}")
sys.exit(app.exec())
