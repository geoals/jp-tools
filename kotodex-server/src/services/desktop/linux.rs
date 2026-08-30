//! The X11 side of [`super`], through `xdotool`.
//!
//! `xdotool` is asked rather than X linked directly because the overlay and
//! `vn-capture.sh` already need it on `PATH` for window geometry, so it is not a
//! dependency this adds. Absent, both calls answer empty and the reader types
//! the title instead.
//!
//! **Every X server, not just this session's.** A game run under gamescope is
//! not on the session's display: gamescope brings up an Xwayland of its own and
//! puts the game on that, so `DISPLAY=:2` inside the sandbox while the session
//! is `:1`. The window is a perfectly ordinary X window — it is simply on an X
//! server nothing here was asking. One `xdotool` run per socket in
//! [`X11_SOCKETS`] finds it, and the display number gamescope picks changes from
//! launch to launch, so the sockets are enumerated rather than configured.
//!
//! Only [`titles`] does this. "Which window is focused" asked of a nested server
//! always answers the game, whether or not the game is what the reader is
//! looking at, so [`focused`] stays on the session's own display where the
//! question means something.

use crate::error::AppError;
use std::collections::BTreeSet;
use std::time::Duration;

/// Where X servers on this machine put their sockets, one `X<n>` per display.
const X11_SOCKETS: &str = "/tmp/.X11-unix";

/// Per-run limit. There was no timeout when this asked one display it knew was
/// alive; asking every socket means asking servers that may be wedged or
/// refusing us, and the picker must not hang on one of them.
const XDOTOOL_TIMEOUT: Duration = Duration::from_secs(3);

pub async fn titles() -> Result<Vec<String>, AppError> {
    let mut names = Vec::new();
    for display in displays() {
        let out = xdotool(&display, &["search", "--name", ".", "getwindowname", "%@"]).await?;
        names.extend(out.lines().map(str::to_string));
    }
    Ok(names)
}

pub async fn focused() -> Result<Option<String>, AppError> {
    let display = std::env::var("DISPLAY").unwrap_or_default();
    let out = xdotool(&display, &["getactivewindow", "getwindowname"]).await?;
    Ok(out.lines().next().map(str::to_string))
}

/// Every display with a socket, as `:0`, `:1`, … and sorted for a stable list.
///
/// A server we are not allowed to open is not a failure — the greeter's `:0`
/// refuses us on an ordinary desktop — it just contributes no titles, which is
/// what [`xdotool`] answers for it anyway.
fn displays() -> Vec<String> {
    let mut found = BTreeSet::new();
    if let Ok(entries) = std::fs::read_dir(X11_SOCKETS) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(number) = name.strip_prefix('X') {
                if !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) {
                    found.insert(format!(":{number}"));
                }
            }
        }
    }
    // No socket directory at all, which is an X server reachable only over TCP
    // or none. The session's own display is still worth the one run.
    if found.is_empty() {
        if let Ok(display) = std::env::var("DISPLAY") {
            if !display.is_empty() {
                found.insert(display);
            }
        }
    }
    found.into_iter().collect()
}

/// One `xdotool` run's stdout, against one display. A missing binary, a failed
/// run and a timeout are the same answer — no titles — because the caller's
/// fallback is the same for all three and an error here would put a red row on a
/// machine that simply has no X.
async fn xdotool(display: &str, args: &[&str]) -> Result<String, AppError> {
    let mut cmd = tokio::process::Command::new("xdotool");
    cmd.env("DISPLAY", display).args(args);
    let out = match tokio::time::timeout(XDOTOOL_TIMEOUT, cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(_)) | Err(_) => return Ok(String::new()),
    };
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
