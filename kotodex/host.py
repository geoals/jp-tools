"""Which module answers for this platform, and the whole list of what it answers.

**The launcher contains no `sys.platform`.** Everything the operating system
decides is one of the names below, and each platform's module holds all of them:
`host_linux.py`, `host_windows.py`. Adding a platform means adding a module and
nothing else; reading what a platform does means reading one file.

- `ROOT`         the install root, whose layout every child is told about
- `LOG_DIR`      where a component's output is appended
- `ICON`         the tray icon
- `components`   the component list, in start order, built from a `Child` class
- `start_dictionary_sync` `jp-dict sync`, started beside the components and
  waited for by nothing
- `stop_port`    stop whatever is listening on a port, whoever started it
- `run_doctor`   show the doctor, or `False` when this platform has none
- `attach_console` make a CLI verb's output visible to whoever ran it
- `apply_identity` what makes the taskbar and tray show Kotodex rather than the
  interpreter
- `spawn_kwargs` / `stop_child`  how a child is detached, and how it is stopped:
  one decision per component, because the Textractor source is frozen with a
  console so a break event can reach it and `TerminateProcess` would be the
  abortive disconnect that crashes Textractor
"""

import sys

if sys.platform == "win32":
    import host_windows as _host
else:
    import host_linux as _host

ROOT = _host.ROOT
LOG_DIR = _host.LOG_DIR
ICON = _host.ICON
components = _host.components
start_dictionary_sync = _host.start_dictionary_sync
stop_port = _host.stop_port
run_doctor = _host.run_doctor
doctor_command = _host.doctor_command
attach_console = _host.attach_console
apply_identity = _host.apply_identity
spawn_kwargs = _host.spawn_kwargs
stop_child = _host.stop_child
