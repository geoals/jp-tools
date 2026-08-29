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

**Nothing here knows which platform it is on.** Which components there are, how
each is started and stopped, and where the assets and logs live are `host`'s —
see its docstring for the whole contract. Windows runs this same launcher.
"""

import os
import signal
import subprocess
import sys
import time
import webbrowser
from pathlib import Path

import config
import host
from config import SOCKET_NAME

# Where the assets are. The binaries are relocatable and the path they were
# compiled in is not, so every child is told rather than left to guess — see
# jp_core::install::install_root. Exported only when the layout is really there,
# because a wrong `KOTODEX_ROOT` wins over the binary's own answer and a missing
# one does not.
if (host.ROOT / "kotodex-server" / "static").is_dir():
    os.environ.setdefault("KOTODEX_ROOT", str(host.ROOT))

# When this process began, as near to it as Python can see — the interpreter's
# own boot is already spent. Every launcher log line is stamped against it, which
# is what makes a slow start readable next to the overlay's own timings.
STARTED = time.monotonic()

# How long the setup probe waits for the server to answer before giving up. A
# cold start with a dictionary import in front of it takes a while, and opening
# nothing is better than opening a page that cannot load.
SETUP_PROBE_SECS = 60

# Restart a child this many times before giving up and saying which one.
MAX_RESTARTS = 3


class Child:
    """One component: how to see it, how to start it, and whether we started it.

    A component that was already running is *adopted* — never started a second
    time, and never stopped on the way out. Stopping something this process did
    not start would take out the user's own setup.
    """

    def __init__(
        self, name, probe, start_cmd, stop_cmd=None, restart_cmd=None,
        ensure_cmd=None, supervised=True, log_file=None,
        stop_adopted=None,
    ):
        self.name = name
        self.probe = probe
        self.start_cmd = start_cmd
        self.stop_cmd = stop_cmd
        # How to bring it back without disturbing one already running — the
        # tray's Show overlay. `None` means starting it is that.
        self.ensure_cmd = ensure_cmd or start_cmd
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
        # Whether the launcher keeps this one alive. The overlay is not
        # supervised: the tray shows and hides it, so it being gone is a state
        # the user chose, not a crash to restart or a reason to quit.
        self.supervised = supervised
        self.proc = None
        self.adopted = False
        self.restarts = 0
        self.failed = False

    def ensure(self, log):
        if self.probe():
            self.adopted = True
            log(f"{self.name}: already running, adopted")
            return
        self.spawn(log)

    def spawn(self, log):
        # Said rather than raised: a component whose binary is absent is a setup
        # that never finished, and a traceback out of the launcher hides which of
        # the three it was.
        exe = self.start_cmd[0]
        if not Path(exe).is_file():
            log(f"{self.name}: {exe} is missing — run setup")
            self.failed = True
            return
        log(f"{self.name}: starting")
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
                cwd=host.ROOT,
                stdout=sink,
                stderr=subprocess.STDOUT if sink is not subprocess.DEVNULL else sink,
                **host.spawn_kwargs(self),
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

    def stop(self, log, force=False):
        """Stop it, unless it is adopted and nobody insisted.

        `force` is the tray's Hide overlay: hiding one this launcher adopted is
        what the reader asked for, where quitting must still leave it alone.
        """
        if self.adopted and not force:
            log(f"{self.name}: left running (it was not ours)")
            return
        if self.stop_cmd:
            subprocess.run(self.stop_cmd, cwd=host.ROOT, capture_output=True)
        if self.proc and self.proc.poll() is None:
            log(f"{self.name}: stopping")
            host.stop_child(self)


def children():
    """This platform's components, in start order. Stopping walks it backwards."""
    return host.components(Child)


def status_report():
    for child in children():
        print(f"{'running' if child.probe() else 'stopped':>8}  {child.name}")


def restart_command() -> int:
    """`kotodex restart` from a terminal.

    Handed to a running launcher when there is one: it supervises kotodex-server,
    and a restart done behind its back looks like a crash for the three seconds
    the port is closed — which it answers by starting a second one.
    """
    from PySide6.QtCore import QCoreApplication

    from single_instance import SingleInstance

    app = QCoreApplication(sys.argv)  # noqa: F841 — QLocalSocket needs an app
    instance = SingleInstance(SOCKET_NAME)
    if not instance.already_running():
        print("kotodex: not running")
        return 0
    instance.send("restart")
    print("kotodex: asked the running launcher to restart everything")
    return 0


def quit_command() -> int:
    """`kotodex quit`, and what the overlay's ✕ reaches.

    Only a running launcher can do this: it is the one that knows which
    components it started and so which ones quitting is allowed to stop.
    """
    from PySide6.QtCore import QCoreApplication

    from single_instance import SingleInstance

    app = QCoreApplication(sys.argv)  # noqa: F841 — QLocalSocket needs an app
    instance = SingleInstance(SOCKET_NAME)
    if not instance.already_running():
        print("kotodex: not running")
        return 0
    instance.send("quit")
    return 0


def restart_running(kids, log, restarting):
    """Stop what this launcher owns, restart what it does not, start again.

    A component this launcher started must come back as its child, or the
    restart would quietly convert it into something adopted — running, but
    no longer stopped on the way out.
    """
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
            subprocess.run(child.restart_cmd, cwd=host.ROOT, capture_output=True)
    for child in kids:
        child.proc = None
        child.adopted = False
        child.restarts = 0
        child.failed = False
        child.ensure(log)
    restarting["until"] = 0.0
    log("restarted")


def main() -> int:
    args = sys.argv[1:]
    # Before anything prints: a frozen GUI build has no console of its own.
    host.attach_console()
    if args and args[0] == "status":
        status_report()
        return 0
    if args and args[0] == "doctor":
        doctor = host.doctor_command()
        if doctor is None:
            print("kotodex: there is no doctor on this platform")
            return 1
        return subprocess.run([*doctor, *args[1:]]).returncode
    if args and args[0] == "restart":
        return restart_command()
    if args and args[0] == "quit":
        return quit_command()
    if args and args[0] == "anki":
        # The field map lives in AnkiConfig, so the check is a Rust binary
        # rather than a second list of field names here.
        binary = host.ROOT / "target" / "release" / "anki-setup"
        if not binary.is_file():
            print(f"{binary} is missing — run setup")
            return 1
        return subprocess.run([str(binary), *args[1:]]).returncode

    from PySide6.QtCore import QTimer
    from PySide6.QtWidgets import QApplication

    from single_instance import SingleInstance
    from tray import Tray

    app = QApplication(sys.argv)
    app.setApplicationName("Kotodex")
    host.apply_identity(app)
    app.setQuitOnLastWindowClosed(False)

    instance = SingleInstance(SOCKET_NAME)
    if instance.already_running():
        # Not an error: launching twice is how someone brings the overlay back.
        instance.send("show")
        return 0

    # Timed like the overlay's own log, so "starting felt slow" is answerable
    # from the two logs side by side rather than guessed at. Everything up to the
    # overlay being spawned is serial, so each line is a phase boundary.
    #
    # To a file as well as stdout: the launcher is frozen --windowed, so on
    # Windows it has no console and stdout goes nowhere at all — these lines, the
    # only record of the launcher's own share of starting, were unobservable on
    # the platform where that share is largest. Appended, and each run says when
    # it began, so a slow start can be read against the one before it.
    log_path = host.LOG_DIR / "kotodex.log"
    log_path.parent.mkdir(parents=True, exist_ok=True)
    log_file = log_path.open("a", buffering=1)
    log_file.write(f"=== {time.strftime('%Y-%m-%d %H:%M:%S')}\n")

    def log(msg):
        line = f"kotodex: +{time.monotonic() - STARTED:6.2f}s {msg}"
        print(line, flush=True)
        log_file.write(f"{line}\n")

    # Started and never waited for. Waiting bought one thing — a zip dropped in
    # since the last start being visible on this one — and the components start in
    # order, so it stood in front of the overlay as well as the server. A new
    # dictionary costs one restart instead. Nothing else here needs it: the reader
    # writes the derived cache itself when it has to derive it.
    if host.start_dictionary_sync() is not None:
        log("jp-dict sync: starting")
    kids = children()

    # Everything at once, and the server's boot reported afterwards rather than
    # waited out in the middle.
    #
    # The overlay used to be held back until the port answered, which put the
    # server's whole boot in front of a third of a second of Python and Qt
    # starting that needs no server at all. It does not need the gate: a page
    # that loads too early retries, and `Overlay.qml` turns Chromium's error page
    # off so a failed load leaves the surface as it was rather than covering the
    # screen with something that cannot be dismissed. Windows already started it
    # this way.
    for child in kids:
        child.ensure(log)

    tray = Tray(app, kids, config.SERVER_URL, log)

    # Set while a restart is in flight, so the watchdog does not read the gap
    # where kotodex-server's port is closed as a crash and start a second one.
    restarting = {"until": 0.0}

    def restart_here():
        restart_running(kids, log, restarting)

    def on_message(msg):
        if msg == "restart":
            restart_here()
        elif msg == "quit":
            app.quit()
        else:
            tray.show_overlay()

    instance.on_message(on_message)
    tray.restart_here = restart_here

    # A fresh install has nothing to read yet, and the only place that says what
    # is missing is the dashboard — which nothing on this path opens. Polled
    # rather than asked once: the server is still booting when the launcher gets
    # here, and the answer before it answers is "not known".
    #
    # `#setup` and not the root, because the root's gate is the same page one
    # click further away for a reader who has never seen either.
    setup_poll = {"until": time.monotonic() + SETUP_PROBE_SECS}

    def probe_setup():
        # The port first: this runs on the Qt thread, and the probe's own request
        # would sit out its timeout on every tick while the server is still
        # booting.
        blocked = config.setup_blocked() if config.kotodex_server_up() else None
        if blocked is None:
            if time.monotonic() < setup_poll["until"]:
                return
            log("setup probe: no answer, not opening the dashboard")
        elif blocked:
            log("setup is blocking — opening the dashboard")
            webbrowser.open(f"{config.SERVER_URL}/#setup")
        setup_probe.stop()

    setup_probe = QTimer()
    setup_probe.timeout.connect(probe_setup)
    setup_probe.start(1500)

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
    sys.exit(main())
