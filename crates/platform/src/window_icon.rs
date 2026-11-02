//! The icon of one of the app's windows (its taskbar button, Alt+Tab),
//! changed while it runs: `WM_SETICON`. The executable keeps its own icon,
//! which Explorer shows for the file and its shortcuts.

use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, HICON, ICON_BIG, ICON_SMALL, SM_CXICON, SM_CXSMICON, WM_SETICON};

use crate::tray::{create_icon, destroy_icon, pick, Icon};

/// Sets the big and the small icon of the window `hwnd` (a number, as raw
/// window handles give it) from `icons`, each at the size it is drawn at for
/// the window's DPI. Call it on the window's thread. The icons it replaces
/// were made here too: they are destroyed.
pub fn set_window_icon(hwnd: isize, icons: &[Icon]) {
    let hwnd = hwnd as HWND;
    // SAFETY: a plain query; 0 for a window that is gone.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    for (kind, metric, fallback) in [(ICON_BIG, SM_CXICON, 32), (ICON_SMALL, SM_CXSMICON, 16)] {
        // SAFETY: a plain query.
        let n = if dpi > 0 { unsafe { GetSystemMetricsForDpi(metric, dpi) } } else { 0 };
        let size = if n > 0 { n as u32 } else { fallback };
        let Some(icon) = pick(icons, size) else { return };
        let hicon = create_icon(icon);
        if hicon.is_null() {
            log::warn!("could not make the window icon: {}", std::io::Error::last_os_error());
            continue;
        }
        // SAFETY: `hicon` is a valid icon; the window answers with the icon
        // it had from WM_SETICON, which only this function sets.
        let old = unsafe { SendMessageW(hwnd, WM_SETICON, kind as WPARAM, hicon as LPARAM) };
        destroy_icon(old as HICON);
    }
}
