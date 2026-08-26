"""The Kotodex launcher: one process that owns the others.

Three things have to be running for a reading session — the capture daemon, the
kotodex-server server, the overlay — and starting them by hand is three terminals
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
import re
import shutil
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
# Where the assets are. The binaries are relocatable and the path they were
# compiled in is not, so every child is told rather than left to guess — see
# jp_core::install::install_root.
os.environ.setdefault("KOTODEX_ROOT", str(REPO))
SERVER_PORT = int(os.environ.get("KOTODEX_SERVER_PORT", "3200"))
SERVER_URL = os.environ.get(
    "KOTODEX_SERVER_URL", f"http://127.0.0.1:{SERVER_PORT}"
)
# Reverse-DNS off kotodex.com, and the same string as the desktop entry's
# filename: on Wayland Qt uses it as the app_id, which is how the compositor
# matches the window to the entry.
APP_ID = "com.kotodex.Kotodex"
SOCKET_NAME = APP_ID

# The three components' entry points, named once. `tray.py` imports these rather
# than rebuilding them, so "where is the overlay script" has one answer.
OVERLAY_SH = str(REPO / "kotodex-server" / "overlay" / "vn-overlay.sh")
SERVER_BIN = REPO / "target" / "release" / "kotodex-server"
DOCTOR_SH = REPO / "scripts" / "kotodex-doctor.sh"
ICON = REPO / "kotodex" / "kotodex.svg"

# How long kotodex-server gets to answer before it is called down. The launcher
# never builds it — see --no-build — so this covers the migrations it runs
# against knowledge.db on the way up, not a compile.
READY_TIMEOUT = 60.0
# How long a restarted capture gets to answer before it is read as down and
# started a second time. Its restart command detaches and returns before the
# daemon it spawned is up.
CAPTURE_READY = 30.0
# A detaching child gets this long to appear before the probe is trusted.
SPAWN_GRACE = 10.0
# Restart a child this many times before giving up and saying which one.
MAX_RESTARTS = 3


def kotodex_server_up() -> bool:
    try:
        with urllib.request.urlopen(f"{SERVER_URL}/api/reader/state", timeout=1) as r:
            return r.status == 200
    except (urllib.error.URLError, OSError):
        return False


def capture_binary() -> str:
    return shutil.which("kotodex-capture") or str(REPO / "capture" / "kotodex-capture")


def capture_up() -> bool:
    """Asked of the daemon's own script, which knows whether systemd owns it.

    Not the ring buffer: a segment is only rewritten every 5s, so a daemon that
    has just died still leaves fresh files behind and gets adopted — running,
    in the launcher's view, with nothing recording.
    """
    return subprocess.run(
        [capture_binary(), "status"], capture_output=True
    ).returncode == 0


def overlay_up() -> bool:
    """Asked of the overlay's own script, which owns the pid file and the lock.

    A bare `pgrep -f vn-overlay.py` matches any command line that merely
    mentions the script — a shell loop, an editor — and reads it as a running
    overlay. There is one answer to this and it is not here.
    """
    return subprocess.run(
        [OVERLAY_SH, "status"], capture_output=True
    ).returncode == 0


class Child:
    """One component: how to see it, how to start it, and whether we started it.

    A component that was already running is *adopted* — never started a second
    time, and never stopped on the way out. Stopping something this process did
    not start would take out the user's own setup.
    """

    def __init__(
        self, name, probe, start_cmd, stop_cmd=None, restart_cmd=None,
        detaches=False, supervised=True, log_file=None, stop_adopted=None
    ):
        self.name = name
        self.probe = probe
        self.start_cmd = start_cmd
        self.stop_cmd = stop_cmd
        # Where this child's output goes. `None` discards it, which is right
        # only for a component whose own log says why it stopped — the overlay
        # script's does.
        self.log_file = log_file
        # How to make an *adopted* one pick up new code. Stopping it is what
        # adoption promises not to do, so this asks it to restart itself.
        self.restart_cmd = restart_cmd or start_cmd
        # For a component that has no "restart yourself" to ask: a bare binary
        # cannot be told, and starting a second one only fails to bind. Set, an
        # explicit restart *takes the component over* — it is stopped here and
        # comes back as this process's child. Quitting still never does that.
        self.stop_adopted = stop_adopted
        # Whether start_cmd *is* the component or merely launches it.
        # vn-overlay.sh backgrounds the real process and returns 0, so its exit
        # status says nothing and the probe is the only thing that knows whether
        # the component is alive. capture and kotodex-server are the process.
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

    def wait_ready(self, log, timeout):
        """Poll the probe until it answers, so a just-restarted component is
        not read as down and started a second time. Returns whether it ever
        answered; a caller that restarted it will fall back to spawning its
        own when it does not."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            if self.probe():
                return True
            time.sleep(1)
        log(f"{self.name}: not answering after {timeout:.0f}s")
        return False

    def spawn(self, log):
        log(f"{self.name}: starting")
        self.started_at = time.time()
        # Appended, not truncated: a component that has already been restarted
        # once this session has its earlier failure in here, which is the thing
        # worth reading.
        sink = subprocess.DEVNULL
        if self.log_file is not None:
            self.log_file.parent.mkdir(parents=True, exist_ok=True)
            sink = self.log_file.open("a")
        try:
            self.proc = subprocess.Popen(
                self.start_cmd,
                cwd=REPO,
                stdout=sink,
                stderr=subprocess.STDOUT if sink is not subprocess.DEVNULL else sink,
                start_new_session=True,
            )
        finally:
            if sink is not subprocess.DEVNULL:
                sink.close()

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
    capture = capture_binary()
    return [
        Child(
            "capture",
            capture_up,
            [capture, "run"],
            restart_cmd=[capture, "restart"],
            # The daemon's own log holds the lines it hooks, not why it refused
            # to start. Without this, a missing dependency is discarded and the
            # reader is left with a status bar saying capture is down.
            log_file=REPO / "logs" / "kotodex-capture.log",
        ),
        Child(
            "kotodex-server",
            kotodex_server_up,
            # The binary directly. It is an ordinary foreground process, so the
            # launcher owns it the way it owns the capture daemon: `stop` is a
            # SIGTERM to its own child and nothing else has to be asked.
            #
            # `scripts/start-all.sh` can run it too, and this adopts one that
            # already answers — but that script is the manual multi-service tool
            # (yt-mine, manga-mine, whisper, the OCR service) and starting one
            # service is not worth taking a dependency on all of them. It also
            # never builds: clicking the desktop entry must not wait on cargo.
            [str(SERVER_BIN)],
            log_file=REPO / "logs" / "kotodex-server.log",
            stop_adopted=lambda: stop_port(SERVER_PORT),
        ),
        Child(
            "overlay",
            overlay_up,
            [OVERLAY_SH, "start"],
            stop_cmd=[OVERLAY_SH, "stop"],
            restart_cmd=[OVERLAY_SH, "restart"],
            detaches=True,
            supervised=False,
        ),
    ]


def wait_for_kotodex_server(log):
    deadline = time.time() + READY_TIMEOUT
    while time.time() < deadline:
        if kotodex_server_up():
            return True
        time.sleep(1)
    log("kotodex-server: no answer — not starting the overlay")
    return False


def status_report():
    for name, probe in (
        ("capture", capture_up),
        ("kotodex-server", kotodex_server_up),
        ("overlay", overlay_up),
    ):
        print(f"{'running' if probe() else 'stopped':>8}  {name}")


def restart_command() -> int:
    """`kotodex restart` from a terminal.

    Handed to a running launcher when there is one: it supervises kotodex-server,
    and a restart done behind its back looks like a crash for the three seconds
    the port is closed — which it answers by starting a second one.
    """
    from PySide6.QtWidgets import QApplication

    from single_instance import SingleInstance

    app = QApplication(sys.argv)  # noqa: F841 — QLocalSocket needs an app
    instance = SingleInstance(SOCKET_NAME)
    if instance.already_running():
        instance.send("restart")
        print("kotodex: asked the running launcher to restart everything")
        return 0
    return restart_components()


def quit_command() -> int:
    """`kotodex quit`, and what the overlay's ✕ reaches.

    Only a running launcher can do this: it is the one that knows which
    components it started and so which ones quitting is allowed to stop.
    """
    from PySide6.QtWidgets import QApplication

    from single_instance import SingleInstance

    app = QApplication(sys.argv)  # noqa: F841 — QLocalSocket needs an app
    instance = SingleInstance(SOCKET_NAME)
    if not instance.already_running():
        print("kotodex: not running")
        return 0
    instance.send("quit")
    return 0


def port_pid(port: int) -> int | None:
    """The pid listening on `port`, or None.

    Resolved from the *port* and never from the process name: `dev-instance.sh`
    runs the same binary from the same path on 3299, so a name match would take
    out a reading session's server while someone worked on a copy of it.
    """
    out = subprocess.run(["ss", "-ltnp"], capture_output=True, text=True)
    for line in out.stdout.splitlines():
        fields = line.split()
        if len(fields) < 4 or not fields[3].endswith(f":{port}"):
            continue
        found = re.search(r"pid=(\d+)", line)
        if found:
            return int(found.group(1))
    return None


def stop_port(port: int) -> None:
    """SIGTERM whatever is listening on `port`, and wait for it to let go."""
    pid = port_pid(port)
    if pid is None:
        return
    try:
        os.kill(pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    for _ in range(20):
        if port_pid(port) is None:
            return
        time.sleep(0.5)


def start_kotodex_server(log=print) -> subprocess.Popen | None:
    """Run the server, its output appended to `logs/kotodex-server.log`."""
    if not SERVER_BIN.is_file():
        log(f"{SERVER_BIN} is missing — run setup.sh")
        return None
    log_path = REPO / "logs" / "kotodex-server.log"
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with log_path.open("a") as sink:
        return subprocess.Popen(
            [str(SERVER_BIN)],
            cwd=REPO,
            stdout=sink,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )


def restart_kotodex_server() -> bool:
    """Stop whatever is serving kotodex-server and start it again.

    Deliberately not adoption-safe, unlike quitting: picking up new code is the
    whole point of a restart, so it has to reach a server this process did not
    start. Nothing else can — `start-all.sh` is the multi-service tool and the
    launcher no longer goes through it.
    """
    stop_port(SERVER_PORT)
    if start_kotodex_server() is None:
        return False
    deadline = time.time() + READY_TIMEOUT
    while time.time() < deadline:
        if kotodex_server_up():
            return True
        time.sleep(1)
    return False


def restart_components() -> int:
    """Pick up new code in everything, whoever started it.

    The launcher cannot do this by adopting: a component it adopted is one it
    deliberately never touches, and the capture daemon is usually systemd's. So
    an update has no way to become live short of knowing which of three things
    started each piece — which is exactly what this hides.
    """
    failed = 0
    print("restarting capture")
    if subprocess.run([capture_binary(), "restart"], cwd=REPO).returncode != 0:
        print("  capture did not restart cleanly")
        failed += 1
    print("restarting kotodex-server")
    if not restart_kotodex_server():
        print("  kotodex-server did not restart cleanly")
        failed += 1
    print("restarting overlay")
    if subprocess.run([OVERLAY_SH, "restart"], cwd=REPO).returncode != 0:
        print("  overlay did not restart cleanly")
        failed += 1
    return 1 if failed else 0


def main() -> int:
    args = sys.argv[1:]
    if args and args[0] == "status":
        status_report()
        return 0
    if args and args[0] == "doctor":
        return subprocess.run([str(DOCTOR_SH), *args[1:]]).returncode
    if args and args[0] == "restart":
        return restart_command()
    if args and args[0] == "quit":
        return quit_command()
    if args and args[0] == "anki":
        # The field map lives in AnkiConfig, so the check is a Rust binary
        # rather than a second list of field names here.
        binary = REPO / "target" / "release" / "anki-setup"
        if not binary.is_file():
            print(f"{binary} is missing — run setup.sh")
            return 1
        return subprocess.run([str(binary), *args[1:]]).returncode

    from PySide6.QtCore import QTimer
    from PySide6.QtWidgets import QApplication

    from single_instance import SingleInstance
    from tray import Tray

    app = QApplication(sys.argv)
    app.setApplicationName("Kotodex")
    # Ties the process to the desktop entry, which is what makes the taskbar
    # and the tray show its name and icon rather than "python3".
    app.setDesktopFileName(APP_ID)
    app.setQuitOnLastWindowClosed(False)

    instance = SingleInstance(SOCKET_NAME)
    if instance.already_running():
        # Not an error: launching twice is how someone brings the overlay back.
        instance.send("show")
        return 0

    def log(msg):
        print(f"kotodex: {msg}", flush=True)

    kids = children()

    # The overlay draws a kotodex-server page, so starting it before the port
    # answers puts a browser error page over the whole screen with no way to
    # dismiss it. If kotodex-server never comes up the tray is how it is retried.
    serving = True
    for child in kids:
        if child.name == "overlay" and not serving:
            continue
        child.ensure(log)
        if child.name == "kotodex-server":
            serving = wait_for_kotodex_server(log)

    tray = Tray(app, kids, SERVER_URL, log)

    # Set while a restart is in flight, so the watchdog does not read the gap
    # where kotodex-server's port is closed as a crash and start a second one.
    restarting = {"until": 0.0}

    def restart_here():
        """Stop what this launcher owns, restart what it does not, start again.

        Not `restart_components`: a component this launcher started must come
        back as its child, or the restart would quietly convert it into
        something adopted — running, but no longer stopped on the way out.
        """
        nonlocal serving
        log("restarting everything")
        restarting["until"] = time.time() + 120
        for child in reversed(kids):
            if not child.adopted:
                child.stop(log)
            elif child.stop_adopted is not None:
                # Nothing to ask: a bare binary has no "restart yourself", and
                # starting a second one only fails to bind. So an explicit
                # restart takes it over — see `Child.stop_adopted`.
                log(f"{child.name}: taking over on restart")
                child.stop_adopted()
            else:
                # Someone else's — a systemd unit, a start-all.sh run. Told to
                # restart itself rather than stopped, since stopping it is
                # exactly what adoption promises not to do.
                subprocess.run(child.restart_cmd, cwd=REPO, capture_output=True)
        for child in kids:
            child.proc = None
            child.adopted = False
            child.restarts = 0
            child.failed = False
            # A restarted child answers its probe later than its restart command
            # returns — the capture daemon's `restart` detaches — so give it
            # time to come back before probing it, or `ensure` reads the restart
            # as a down component and starts a second one. kotodex-server has no such
            # gap: it was stopped above and `ensure` spawns it here.
            if child.name == "capture":
                child.wait_ready(log, CAPTURE_READY)
            if child.name == "overlay" and not serving:
                continue
            child.ensure(log)
            if child.name == "kotodex-server":
                serving = wait_for_kotodex_server(log)
        restarting["until"] = 0.0
        log("restarted")

    def on_message(msg):
        if msg == "restart":
            restart_here()
        elif msg == "quit":
            app.quit()
        else:
            tray.show_overlay()

    instance.on_message(on_message)
    tray.restart_here = restart_here

    def tick():
        if time.time() < restarting["until"]:
            return
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
