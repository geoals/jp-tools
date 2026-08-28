"""What the launcher's platform answers for, on Windows.

See `host.py` for the contract. This is the whole of the Windows-specific
launcher: three components rather than Linux's three — kotodex-server, the
Textractor source and the overlay — because capture is audio and Linux-only, and
so is the doctor.
"""

import os
import signal
import subprocess
import sys
import time
from pathlib import Path

import config

# Frozen, the launcher is `<root>\launcher\kotodex.exe`; from a checkout this file
# is `<root>\kotodex\host_windows.py`. Both are two levels down, and the root is
# what every child is told about.
if getattr(sys, "frozen", False):
    ROOT = Path(sys.executable).resolve().parents[1]
else:
    ROOT = Path(__file__).resolve().parents[1]

# Beside the databases, which is where setup.ps1, the launcher's logs and the
# uninstaller already agree they are. Under `{app}` they would be files the
# installer does not own and an upgrade would delete.
LOG_DIR = Path(os.environ["LOCALAPPDATA"]) / "kotodex"
ICON = ROOT / "kotodex" / "icons" / "kotodex.ico"

SERVER_EXE = ROOT / "target" / "release" / "kotodex-server.exe"
SOURCE_EXE = ROOT / "source" / "kotodex-source.exe"
OVERLAY_EXE = ROOT / "overlay" / "kotodex-overlay.exe"

# The one component that is stopped by being told rather than killed.
SOURCE = "source"

CREATE_NEW_PROCESS_GROUP = 0x00000200
CREATE_NO_WINDOW = 0x08000000


def _quietly(*command) -> subprocess.CompletedProcess:
    return subprocess.run(
        command, capture_output=True, text=True, creationflags=CREATE_NO_WINDOW
    )


def _running(exe: Path) -> bool:
    """Whether anything is running from that executable, by image name.

    Every one of these is named after this application, unlike python.exe, which
    anything on the machine may be running.
    """
    found = _quietly("tasklist", "/NH", "/FI", f"IMAGENAME eq {exe.name}")
    return exe.name.lower() in found.stdout.lower()


def components(Child):
    """In start order. Stopping walks it backwards."""
    return [
        Child(
            "kotodex-server",
            config.kotodex_server_up,
            [str(SERVER_EXE)],
            log_file=LOG_DIR / "kotodex-server.log",
            stop_adopted=lambda: stop_port(config.SERVER_PORT),
        ),
        Child(
            SOURCE,
            lambda: _running(SOURCE_EXE),
            [str(SOURCE_EXE)],
            log_file=LOG_DIR / "kotodex-source.log",
        ),
        Child(
            "overlay",
            lambda: _running(OVERLAY_EXE),
            [str(OVERLAY_EXE)],
            # Whoever started it: the tray's Hide overlay is allowed to stop an
            # overlay this launcher adopted, and there is no wrapper script here
            # holding a pid file to ask.
            stop_cmd=["taskkill", "/IM", OVERLAY_EXE.name, "/F"],
            log_file=LOG_DIR / "kotodex-overlay.log",
            supervised=False,
        ),
    ]


def port_pid(port: int) -> int | None:
    """The pid listening on `port`, or None.

    From the port and never a process name, for the reason the Linux one says:
    a second copy of the same binary on another port is somebody's dev instance.
    """
    found = _quietly("netstat", "-ano", "-p", "TCP")
    for line in found.stdout.splitlines():
        fields = line.split()
        if len(fields) < 5 or fields[3] != "LISTENING":
            continue
        if fields[1].endswith(f":{port}"):
            return int(fields[4])
    return None


def stop_port(port: int) -> None:
    """Stop whatever is listening on `port`, and wait for it to let go.

    Killed rather than asked: there is no SIGTERM here, and a console process
    with no window ignores the close message `taskkill` sends without `/F`. The
    databases are WAL and the installer's own uninstall step does the same.
    """
    pid = port_pid(port)
    if pid is None:
        return
    _quietly("taskkill", "/PID", str(pid), "/T", "/F")
    for _ in range(20):
        if port_pid(port) is None:
            return
        time.sleep(0.5)


def run_doctor() -> bool:
    return False


def doctor_command() -> None:
    return None


def attach_console() -> None:
    """Write to the console that launched us, where there is one.

    The frozen launcher is a GUI application and has no console of its own, so
    `kotodex.exe status` printed nowhere. Skipped when the streams are already
    somewhere — a caller capturing the output has redirected them, and a GUI
    launch has no console to attach to.
    """
    import ctypes

    if sys.stdout is not None:
        return
    if not ctypes.windll.kernel32.AttachConsole(-1):
        return
    sys.stdout = open("CONOUT$", "w", buffering=1, encoding="utf-8", errors="replace")
    sys.stderr = sys.stdout


def apply_identity(app) -> None:
    """Name the process to the shell, so the taskbar groups it as Kotodex and
    shows its icon rather than a generic entry."""
    import ctypes

    ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID(config.APP_ID)


def spawn_kwargs(child) -> dict:
    """No window for anything, and a console the source can be signalled through.

    `CREATE_NO_WINDOW` does not hide a console — it leaves the process without
    one, and a process with no console receives no `CTRL_BREAK_EVENT`. That event
    is the source's only chance to send Textractor's plugin a proper close frame,
    and an abortive disconnect crashes Textractor itself. So the source gets a
    real console with its window hidden through `STARTUPINFO`, and a process group
    of its own for the event to be sent to. Under `CREATE_NO_WINDOW` it ignores
    the break event and has to be killed; this way it exits with 0 in a third of a
    second.
    """
    if child.name != SOURCE:
        return {"creationflags": CREATE_NO_WINDOW}
    hidden = subprocess.STARTUPINFO()
    hidden.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    hidden.wShowWindow = subprocess.SW_HIDE
    return {"creationflags": CREATE_NEW_PROCESS_GROUP, "startupinfo": hidden}


def _break(pid: int) -> bool:
    """Send `CTRL_BREAK_EVENT` to another process's console.

    Always to the child's own process group, which is what it was given one for.
    Never to the whole console: the launcher may be sharing that console, and a
    sender cannot exempt itself — `SetConsoleCtrlHandler(NULL, TRUE)` ignores
    CTRL+C alone, so the launcher stopped itself half way through its own
    shutdown and left the server running.

    `Popen.send_signal` cannot do it either: `GenerateConsoleCtrlEvent` reaches
    only a process sharing the *caller's* console, and a windowless Qt
    application has none. So where the direct call fails, the child's console is
    borrowed for the length of it.
    """
    import ctypes

    k = ctypes.windll.kernel32
    if k.GenerateConsoleCtrlEvent(signal.CTRL_BREAK_EVENT, pid):
        return True
    if not k.AttachConsole(pid):
        return False
    try:
        return bool(k.GenerateConsoleCtrlEvent(signal.CTRL_BREAK_EVENT, pid))
    finally:
        k.FreeConsole()


def stop_child(child) -> None:
    """Told, where being told is what a component needs; killed otherwise.

    The fall-through is not a formality: a source that has to be killed is the
    abortive disconnect that takes Textractor with it, and that is what happens
    when the break event cannot be delivered.
    """
    if child.name == SOURCE and _break(child.proc.pid):
        try:
            child.proc.wait(timeout=5)
            return
        except subprocess.TimeoutExpired:
            pass
    child.proc.terminate()
    try:
        child.proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        child.proc.kill()
