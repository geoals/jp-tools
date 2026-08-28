//! The X11 side of [`super`], through `xdotool`.
//!
//! `xdotool` is asked rather than X linked directly because the overlay and
//! `vn-capture.sh` already need it on `PATH` for window geometry, so it is not a
//! dependency this adds. Absent, both calls answer empty and the reader types
//! the title instead.

use crate::error::AppError;

pub async fn titles() -> Result<Vec<String>, AppError> {
    let out = xdotool(&["search", "--name", ".", "getwindowname", "%@"]).await?;
    Ok(out.lines().map(str::to_string).collect())
}

pub async fn focused() -> Result<Option<String>, AppError> {
    let out = xdotool(&["getactivewindow", "getwindowname"]).await?;
    Ok(out.lines().next().map(str::to_string))
}

/// One `xdotool` run's stdout. A missing binary and a failed run are the same
/// answer — no titles — because the caller's fallback is the same either way and
/// an error here would put a red row on a machine that simply has no X.
async fn xdotool(args: &[&str]) -> Result<String, AppError> {
    let out = tokio::process::Command::new("xdotool")
        .args(args)
        .output()
        .await;
    Ok(match out {
        Ok(out) => String::from_utf8_lossy(&out.stdout).into_owned(),
        Err(_) => String::new(),
    })
}
