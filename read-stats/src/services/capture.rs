//! Firing vn-mine's capture script, and finding the window to point it at.
//!
//! read-stats does not record audio or take screenshots; `vn-capture.sh` does,
//! on the machine running the VN. This module is the boundary: build the
//! environment the script expects, run it, parse the one JSON object it prints.
//!
//! One caller: the auto-capture on card add, which every mine goes through. It
//! relies on the lookup that must have preceded it — Yomitan's popup, or the
//! overlay's.

use std::time::Duration;

use serde_json::Value;
use tracing::{info, warn};

use crate::app::AppState;
use crate::db;
use crate::error::AppError;

/// vn-capture.sh runs VAD and (usually) a whisper transcription for the
/// sentence trim, so it is slow by design. Past this it is stuck, not working.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(90);

/// What the capture is *for*, when the caller knows more than "capture now".
///
/// The mine button knows neither field — it fires against whatever line is
/// current and attaches to the last note added, which is what pressing it
/// means. The card-add path knows both and has to say so, because by the time
/// the script runs the reader may have moved on.
#[derive(Default)]
pub struct Target {
    /// Epoch seconds to resolve "the current line" as of, so reading on while
    /// the capture works cannot pull the audio window onto the next line.
    pub anchor_ts: Option<f64>,
    /// The note to attach to, when the caller created it and knows its id.
    /// Without one the script falls back to the most recently added note, which
    /// is only the right answer while nothing else is added in between.
    pub note_id: Option<i64>,
}

/// Run vn-capture.sh once and return its parsed JSON result.
///
/// A failed capture is a normal outcome (a stale line, Anki closed) and comes
/// back as `{"ok": false, ...}` rather than as an error — the reader shows the
/// message and you press again. `Err` is reserved for the script not running at
/// all.
pub async fn run(state: &AppState, target: Target) -> Result<Value, AppError> {
    let script = state.vn_capture_script.clone();
    if !script.is_file() {
        return Err(AppError::BadRequest(format!(
            "vn-capture.sh not found at {} (set KOTODEX_VN_CAPTURE_SH)",
            script.display()
        )));
    }

    // Which window to screenshot. Without it the script grabs whatever has
    // focus, which is the browser `#read` is open in, not the VN.
    let vn_window = vn_window(state).await;

    let mut cmd = tokio::process::Command::new(&script);
    // The script normally reports through notify-send on the desktop it runs
    // on, which is not necessarily where the mine came from; VN_JSON=1 makes it
    // print a result object instead, and the reader shows that.
    cmd.env("VN_JSON", "1");
    // Left unset when empty so a VN_WINDOW inherited from the environment
    // still applies.
    if !vn_window.is_empty() {
        cmd.env("VN_WINDOW", &vn_window);
    }
    // Six decimals: the same shape vn-ws-logger.py writes into lines.log, and
    // finer than any gap between two hooked lines.
    if let Some(ts) = target.anchor_ts {
        cmd.env("VN_ANCHOR_TS", format!("{ts:.6}"));
    }
    if let Some(id) = target.note_id {
        cmd.env("VN_NOTE_ID", id.to_string());
    }
    let out = match tokio::time::timeout(CAPTURE_TIMEOUT, cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            return Err(AppError::Upstream(format!(
                "could not run vn-capture.sh: {e}"
            )));
        }
        Err(_) => {
            return Err(AppError::Upstream(format!(
                "vn-capture.sh timed out after {}s",
                CAPTURE_TIMEOUT.as_secs()
            )));
        }
    };

    let stdout = String::from_utf8_lossy(&out.stdout);
    let Some(parsed) = stdout.lines().rev().find_map(|l| {
        serde_json::from_str::<Value>(l)
            .ok()
            .filter(Value::is_object)
    }) else {
        // No parseable result: surface the script's own diagnostics, which is
        // all there is to go on (a missing dependency, a broken ring buffer).
        let stderr = String::from_utf8_lossy(&out.stderr);
        let detail = [stderr.trim(), stdout.trim()]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or("no output")
            .lines()
            .next_back()
            .unwrap_or("no output")
            .to_string();
        return Err(AppError::Upstream(format!(
            "vn-capture.sh failed: {detail}"
        )));
    };

    if parsed.get("ok").and_then(Value::as_bool) == Some(true) {
        info!(result = %parsed, "vn-capture succeeded");
    } else {
        warn!(result = %parsed, "vn-capture reported failure");
    }
    Ok(parsed)
}

/// Which window is the VN, as everything that needs to know resolves it.
///
/// The current work's own window first, then the global `vn_window` setting,
/// which is a legacy fallback for setups that predate per-work windows. Empty
/// when neither is set.
///
/// One implementation because there are three callers and they must not
/// disagree: this module, the reader's status event, and `vn-capture.sh` over
/// `GET /api/vn/window`. Two places to say which window is the game is the one
/// thing the per-work column exists to stop — the one you forget points at the
/// last VN.
pub async fn vn_window(state: &AppState) -> String {
    let settings = db::load_settings(&state.local).await.unwrap_or_else(|e| {
        warn!(error = %e, "vn window: settings unreadable");
        Default::default()
    });
    vn_window_for(state, &settings).await
}

/// The same answer for a caller that has already loaded the settings.
///
/// The reader's status event has: it is published every two seconds per open
/// surface, so loading them twice for one event is a query per surface per
/// second for nothing.
pub async fn vn_window_for(state: &AppState, settings: &db::Settings) -> String {
    match db::current_work_vn_window(&state.knowledge, &settings.current_work).await {
        Ok(Some(w)) if !w.trim().is_empty() => w,
        Ok(_) => settings.vn_window.clone(),
        Err(e) => {
            warn!(error = %e, "vn window: the work's own is unreadable");
            settings.vn_window.clone()
        }
    }
}

/// Candidate window titles for the `vn_window` setting.
///
/// The VN's window title can't be guessed from the work title (`素晴らしき日々`
/// vs `素晴らしき日々～不連続存在～`) and changes with every game, so the
/// dashboard offers a list to pick from rather than a blank text box.
pub async fn list_windows() -> Result<Vec<String>, AppError> {
    let out = tokio::process::Command::new("xdotool")
        .args(["search", "--name", ".", "getwindowname", "%@"])
        .output()
        .await
        .map_err(|e| AppError::Upstream(format!("xdotool unavailable: {e}")))?;

    let mut names: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|n| !n.is_empty() && !is_helper_window(n))
        .map(str::to_string)
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// Wine/Qt/IME scaffolding that is never the VN. Everything else is offered,
/// since guessing which of the real windows is the game is the user's call.
fn is_helper_window(name: &str) -> bool {
    const NOISE: &[&str] = &[
        "Default IME",
        "Input",
        "xsettingsd",
        "Chromium clipboard",
        "Fcitx5 Input Window",
    ];
    NOISE.contains(&name) || name.starts_with("Qt Selection Owner")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_windows_are_filtered_but_real_ones_are_not() {
        assert!(is_helper_window("Default IME"));
        assert!(is_helper_window("Qt Selection Owner for wine"));
        assert!(!is_helper_window(
            "ドーナドーナ いっしょにわるいことをしよう"
        ));
    }
}
