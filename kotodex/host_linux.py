"""What the launcher's platform answers for, on Linux.

See `host.py` for the contract. Nothing here is imported directly — the launcher
and the tray go through `host`.
"""

import os
import re
import shutil
import signal
import subprocess
import time
from pathlib import Path

import config

ROOT = Path(__file__).resolve().parents[1]
LOG_DIR = ROOT / "logs"
ICON = ROOT / "kotodex" / "kotodex.svg"

# The three components' entry points, named once.
OVERLAY_SH = str(ROOT / "kotodex-server" / "overlay" / "vn-overlay.sh")
SERVER_BIN = ROOT / "target" / "release" / "kotodex-server"
DOCTOR_SH = ROOT / "scripts" / "kotodex-doctor.sh"

# How long a restarted capture gets to answer before it is read as down and
# started a second time. Its restart command detaches and returns before the
# daemon it spawned is up.
CAPTURE_READY = 30.0


def capture_binary() -> str:
    return shutil.which("kotodex-capture") or str(ROOT / "capture" / "kotodex-capture")


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


def components(Child):
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
            log_file=LOG_DIR / "kotodex-capture.log",
            wait_after_restart=CAPTURE_READY,
        ),
        Child(
            "kotodex-server",
            config.kotodex_server_up,
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
            log_file=LOG_DIR / "kotodex-server.log",
            stop_adopted=lambda: stop_port(config.SERVER_PORT),
        ),
        Child(
            "overlay",
            overlay_up,
            [OVERLAY_SH, "start"],
            stop_cmd=[OVERLAY_SH, "stop"],
            restart_cmd=[OVERLAY_SH, "restart"],
            ensure_cmd=[OVERLAY_SH, "ensure"],
            detaches=True,
            supervised=False,
            # It draws a kotodex-server page, and a browser error page over the
            # whole screen has no way to be dismissed.
            needs_server=True,
        ),
    ]


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


def run_doctor() -> bool:
    """Show the doctor in a terminal. False when there is none to show it in.

    `--` for the terminals that want it and `-e` for the one that does not:
    gnome-terminal dropped `-e` and konsole never took `--`, so each gets the
    form it accepts rather than one form that half of them refuse.
    """
    for term, flag in (
        ("konsole", "-e"),
        ("gnome-terminal", "--"),
        ("xfce4-terminal", "-x"),
        ("xterm", "-e"),
    ):
        if shutil.which(term) is None:
            continue
        subprocess.Popen([term, flag, "bash", "-c", f"{DOCTOR_SH}; read -r"])
        return True
    return False


def doctor_command() -> list[str]:
    """`kotodex doctor` from a terminal that is already there."""
    return [str(DOCTOR_SH)]


def attach_console() -> None:
    """Nothing to do: a CLI verb here already has the terminal's streams."""


def apply_identity(app) -> None:
    """Tie the process to the desktop entry, which is what makes the taskbar and
    the tray show its name and icon rather than "python3"."""
    app.setDesktopFileName(config.APP_ID)


def spawn_kwargs(child) -> dict:
    return {"start_new_session": True}


def stop_child(child) -> None:
    """SIGTERM, then SIGKILL. `child.proc` is live and ours."""
    child.proc.terminate()
    try:
        child.proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        child.proc.kill()
