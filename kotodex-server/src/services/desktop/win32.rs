//! The Windows side of [`super`], through `EnumWindows`.
//!
//! **This is the only Windows-specific file in kotodex-server.** Everything the
//! platform decides about windows is one of the two names below.

use std::ffi::c_void;

use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible,
};

/// Every top-level window with a title a reader could pick.
///
/// Called straight rather than through `spawn_blocking`: `GetWindowTextW` reads
/// another process's *cached* title and sends it no message, so unlike
/// `WM_GETTEXT` it cannot be held up by a game that has stopped pumping — which
/// is exactly the window being asked about here.
pub async fn titles() -> Result<Vec<String>, super::AppError> {
    let mut found: Vec<String> = Vec::new();
    // SAFETY: `collect` is handed a pointer to `found`, which outlives the call
    // because EnumWindows runs every callback before it returns.
    unsafe {
        EnumWindows(Some(collect), &mut found as *mut Vec<String> as LPARAM);
    }
    Ok(found)
}

pub async fn focused() -> Result<Option<String>, super::AppError> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        return Ok(None);
    }
    Ok(title_of(hwnd))
}

/// Returns nonzero to keep enumerating; zero would stop at the first window.
unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> i32 {
    // SAFETY: the `lparam` EnumWindows was given, on the thread that gave it.
    let found = unsafe { &mut *(lparam as *mut Vec<String>) };
    if let Some(title) = title_of(hwnd) {
        found.push(title);
    }
    1
}

/// A window's title, or nothing when it is not one the reader can see.
///
/// Cloaked windows are dropped as well as invisible ones. A packaged app keeps
/// its window alive, visible and titled while it is not on screen at all, so
/// visibility alone fills the picker with Settings, Search and the text-input
/// host — and a picker full of things that are not the game is what this exists
/// to fix.
fn title_of(hwnd: HWND) -> Option<String> {
    if unsafe { IsWindowVisible(hwnd) } == 0 || cloaked(hwnd) {
        return None;
    }
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return None;
    }
    // One over, for the terminator GetWindowTextW writes and does not count.
    let mut buf = vec![0u16; len as usize + 1];
    let written = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if written <= 0 {
        return None;
    }
    buf.truncate(written as usize);
    Some(String::from_utf16_lossy(&buf))
}

fn cloaked(hwnd: HWND) -> bool {
    let mut cloaked: u32 = 0;
    let hr = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            &mut cloaked as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        )
    };
    hr == 0 && cloaked != 0
}
