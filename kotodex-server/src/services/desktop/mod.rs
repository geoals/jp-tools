//! Which windows are open, and which one is in front.
//!
//! **One module per platform, and nothing above this line knows which it got**
//! — the same bargain `kotodex/host.py` makes. Two names in the contract:
//!
//! - [`open_windows`] every title a reader could mean by "the game"
//! - [`focused_window`] the one in front, so picking it is a button rather than
//!   a title typed by hand
//!
//! Both answer `Ok(empty)` rather than `Err` where the platform has no way to
//! ask: a picker with nothing in it falls back to the text box, and a fault row
//! for a machine that cannot have the feature reads as a broken install.

use crate::error::AppError;

#[cfg(windows)]
mod win32;
#[cfg(windows)]
use win32 as host;

#[cfg(not(windows))]
mod linux;
#[cfg(not(windows))]
use linux as host;

/// Candidate window titles for the `vn_window` setting, sorted and deduplicated.
///
/// The VN's window title can't be guessed from the work title (`素晴らしき日々`
/// vs `素晴らしき日々～不連続存在～`) and changes with every game, so the
/// dashboard offers a list to pick from rather than a blank text box.
pub async fn open_windows() -> Result<Vec<String>, AppError> {
    let mut names: Vec<String> = host::titles()
        .await?
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty() && !is_helper_window(n))
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// The window in front, or nothing when it cannot be asked or is scaffolding.
pub async fn focused_window() -> Result<Option<String>, AppError> {
    Ok(host::focused()
        .await?
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty() && !is_helper_window(n)))
}

/// Scaffolding that is never the VN, on either platform. Everything else is
/// offered — guessing which of the real windows is the game is the reader's call.
///
/// One list rather than one per platform: a window called `Program Manager` is
/// not the game under Wine either, and two lists would be two places to add a
/// name to.
fn is_helper_window(name: &str) -> bool {
    const NOISE: &[&str] = &[
        // gamescope's own compositor, which shares the nested display with the
        // game it is scaling.
        "steamcompmgr",
        // Wine, Qt and the input methods.
        "Default IME",
        "Input",
        "xsettingsd",
        "Chromium clipboard",
        "Fcitx5 Input Window",
        // The Windows shell's own always-open windows.
        "Program Manager",
        "Windows Input Experience",
        "Microsoft Text Input Application",
        "Settings",
        "Windows Shell Experience Host",
        "Search",
        "Start",
        "NVIDIA GeForce Overlay",
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
        assert!(is_helper_window("Program Manager"));
        assert!(is_helper_window("steamcompmgr"));
        assert!(!is_helper_window(
            "ドーナドーナ いっしょにわるいことをしよう"
        ));
    }
}
