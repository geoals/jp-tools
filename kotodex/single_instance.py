"""One Kotodex at a time, and a second launch that raises the first.

A desktop entry is clicked twice as often as it is clicked once. The second
launch is not an error and must not print one: it connects to the first, says
what it wants, and exits 0.
"""

from PySide6.QtCore import QObject
from PySide6.QtNetwork import QLocalServer, QLocalSocket

CONNECT_TIMEOUT_MS = 300


class SingleInstance(QObject):
    def __init__(self, name: str):
        super().__init__()
        self.name = name
        self.server = None
        self._handler = None
        self._probe = QLocalSocket()
        self._probe.connectToServer(name)
        self._running = self._probe.waitForConnected(CONNECT_TIMEOUT_MS)
        if not self._running:
            self._listen()

    def _listen(self):
        self.server = QLocalServer()
        # A process killed with SIGKILL leaves its socket file behind, and
        # listen() then fails on a socket nothing is on the other end of.
        # Nothing answered the probe above, so removing it is safe here and
        # only here.
        QLocalServer.removeServer(self.name)
        self.server.listen(self.name)
        self.server.newConnection.connect(self._accept)

    def _accept(self):
        conn = self.server.nextPendingConnection()
        if conn is None:
            return

        def read():
            message = bytes(conn.readAll()).decode(errors="replace").strip()
            if message and self._handler:
                self._handler(message)
            conn.close()

        conn.readyRead.connect(read)

    def already_running(self) -> bool:
        return self._running

    def send(self, message: str):
        if not self._running:
            return
        self._probe.write(message.encode())
        self._probe.flush()
        self._probe.waitForBytesWritten(CONNECT_TIMEOUT_MS)
        self._probe.disconnectFromServer()

    def on_message(self, handler):
        self._handler = handler
