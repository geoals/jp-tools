"""The Kotodex launcher: one process that owns the others.

Three things have to be running for a reading session — the capture daemon, the
read-stats server, the overlay — and starting them by hand is three terminals
and an order to remember. This starts them in that order, keeps them up, and
stops them together.

**Adopt, don't duplicate.** Every component is probed before it is started: a
port already answering, a ring buffer already being written, an overlay already
on screen. That is what lets this coexist with `scripts/start-all.sh` and with a
systemd-managed capture daemon, and it is why closing the launcher stops only
what the launcher started.

Qt because of the tray: a launcher that hides the overlay has to leave something
behind to bring it back. The overlay stays a separate process — it is a layer
surface driven by QML, and merging a widgets tray into it buys nothing.
"""

import os
import shutil
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
READ_STATS_URL = os.environ.get("KOTODEX_READ_STATS_URL", "http://127.0.0.1:3200")
SOCKET_NAME = "kotodex"

# How long read-stats gets to answer before the overlay is started anyway. It
# builds on first run, which is slow and not a failure.
READY_TIMEOUT = 90.0
# A detaching child gets this long to appear before the probe is trusted.
SPAWN_GRACE = 10.0
# Restart a child this many times before giving up and saying which one.
MAX_RESTARTS = 3


def run_dir() -> Path:
    base = os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.getuid()}"
    return Path(base) / "vn-mine"


def read_stats_up() -> bool:
    try:
        with urllib.request.urlopen(f"{READ_STATS_URL}/api/reader/state", timeout=1) as r:
            return r.status == 200
    except (urllib.error.URLError, OSError):
        return False


def capture_up() -> bool:
    """A segment written in the last half minute. ffmpeg rewrites one every 5s,
    so stale files mean the daemon is gone even though the ring is still there."""
    seg = run_dir() / "seg"
    try:
        newest = max((p.stat().st_mtime for p in seg.glob("seg*.wav")), default=0)
    except OSError:
        return False
    return time.time() - newest < 30


def overlay_up() -> bool:
    return subprocess.run(
        ["pgrep", "-f", "vn-overlay.py"], capture_output=True
    ).returncode == 0


class Child:
    """One component: how to see it, how to start it, and whether we started it.

    A component that was already running is *adopted* — never started a second
    time, and never stopped on the way out. Stopping something this process did
    not start would take out the user's own setup.
    """

    def __init__(
        self, name, probe, start_cmd, stop_cmd=None, detaches=False, supervised=True
    ):
        self.name = name
        self.probe = probe
        self.start_cmd = start_cmd
        self.stop_cmd = stop_cmd
        # Whether start_cmd *is* the component or merely launches it.
        # start-all.sh and vn-overlay.sh background the real process and return
        # 0, so for those the exit status says nothing and the probe is the
        # only thing that knows whether the component is alive.
        self.detaches = detaches
        # Whether the launcher keeps this one alive. The overlay is not
        # supervised: the tray shows and hides it, so it being gone is a state
        # the user chose, not a crash to restart or a reason to quit.
        self.supervised = supervised
        self.proc = None
        self.adopted = False
        self.restarts = 0
        self.failed = False
        self.started_at = 0.0

    def ensure(self, log):
        if self.probe():
            self.adopted = True
            log(f"{self.name}: already running, adopted")
            return
        self.spawn(log)

    def spawn(self, log):
        log(f"{self.name}: starting")
        self.started_at = time.time()
        self.proc = subprocess.Popen(
            self.start_cmd,
            cwd=REPO,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )

    def check(self, log) -> bool:
        """Restart a child that exited on its own, with a backoff, and give up
        loudly rather than spinning. Returns True when everything should stop.
        """
        if self.adopted or self.proc is None or self.failed or not self.supervised:
            return False
        if self.detaches:
            # The wrapper returns before the process it launched is visible, so
            # the probe cannot be believed until the component has had time to
            # come up.
            if time.time() - self.started_at < SPAWN_GRACE:
                return False
            if self.proc.poll() is None or self.probe():
                return False
            return self._restart(log)
        code = self.proc.poll()
        if code is None:
            return False
        if code == 0:
            log(f"{self.name}: closed")
            return True
        return self._restart(log)

    def _restart(self, log) -> bool:
        self.restarts += 1
        if self.restarts > MAX_RESTARTS:
            self.failed = True
            log(f"{self.name}: exited {MAX_RESTARTS} times, giving up")
            return False
        time.sleep(min(2**self.restarts, 10))
        log(f"{self.name}: exited, restarting ({self.restarts}/{MAX_RESTARTS})")
        self.spawn(log)
        return False

    def stop(self, log):
        if self.adopted:
            log(f"{self.name}: left running (it was not ours)")
            return
        if self.stop_cmd:
            subprocess.run(self.stop_cmd, cwd=REPO, capture_output=True)
        if self.proc and self.proc.poll() is None:
            log(f"{self.name}: stopping")
            self.proc.terminate()
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.kill()


def children():
    """In start order. Stopping walks it backwards."""
    capture = shutil.which("kotodex-capture") or str(REPO / "vn-mine" / "kotodex-capture")
    overlay = str(REPO / "read-stats" / "overlay" / "vn-overlay.sh")
    return [
        Child("capture", capture_up, [capture, "run"]),
        Child(
            "read-stats",
            read_stats_up,
            [str(REPO / "scripts" / "start-all.sh"), "read-stats"],
            detaches=True,
        ),
        Child(
            "overlay",
            overlay_up,
            [overlay, "start"],
            stop_cmd=[overlay, "stop"],
            detaches=True,
            supervised=False,
        ),
    ]


def wait_for_read_stats(log):
    deadline = time.time() + READY_TIMEOUT
    while time.time() < deadline:
        if read_stats_up():
            return True
        time.sleep(1)
    log("read-stats: no answer yet — starting the overlay anyway")
    return False


def status_report():
    for name, probe in (
        ("capture", capture_up),
        ("read-stats", read_stats_up),
        ("overlay", overlay_up),
    ):
        print(f"{'running' if probe() else 'stopped':>8}  {name}")


def main() -> int:
    args = sys.argv[1:]
    if args and args[0] == "status":
        status_report()
        return 0
    if args and args[0] == "doctor":
        return subprocess.run([str(REPO / "scripts" / "kotodex-doctor.sh")]).returncode

    from PySide6.QtCore import QTimer
    from PySide6.QtWidgets import QApplication

    from single_instance import SingleInstance
    from tray import Tray

    app = QApplication(sys.argv)
    app.setApplicationName("Kotodex")
    # Ties the process to kotodex.desktop, which is what makes the taskbar and
    # the tray show this entry's name and icon rather than "python3".
    app.setDesktopFileName("kotodex")
    app.setQuitOnLastWindowClosed(False)

    instance = SingleInstance(SOCKET_NAME)
    if instance.already_running():
        # Not an error: launching twice is how someone brings the overlay back.
        instance.send("show")
        return 0

    def log(msg):
        print(f"kotodex: {msg}", flush=True)

    kids = children()

    for child in kids:
        child.ensure(log)
        if child.name == "read-stats":
            wait_for_read_stats(log)

    tray = Tray(app, kids, READ_STATS_URL, log)
    instance.on_message(lambda msg: tray.show_overlay() if msg == "show" else None)

    def tick():
        if any(child.check(log) for child in kids):
            app.quit()

    watchdog = QTimer()
    watchdog.timeout.connect(tick)
    watchdog.start(3000)

    def shutdown():
        for child in reversed(kids):
            child.stop(log)

    app.aboutToQuit.connect(shutdown)
    # Ctrl-C in the terminal that launched it should stop everything too.
    signal.signal(signal.SIGINT, lambda *_: app.quit())
    nudge = QTimer()
    nudge.start(200)
    nudge.timeout.connect(lambda: None)

    return app.exec()


if __name__ == "__main__":
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    sys.exit(main())
