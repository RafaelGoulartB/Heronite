//! Native placement and focus for GPUI's Windows overlay surfaces.

use std::io;
use std::num::NonZeroIsize;

use windows_sys::Win32::Foundation::{GetLastError, SetLastError, HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsWindow, SetForegroundWindow,
    SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE, GWL_STYLE, HWND_TOPMOST, LWA_ALPHA,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SW_SHOWNOACTIVATE, WS_CAPTION, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TRANSPARENT, WS_THICKFRAME,
};

/// Physical monitor coordinates and its logical-to-physical scale.
#[derive(Clone, Copy, Debug)]
pub struct Screen {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f32,
}

/// The foreground window's monitor, including negative desktop origins.
pub fn foreground_screen() -> io::Result<Screen> {
    // SAFETY: all handles are queried from Windows; the output structures are initialized.
    unsafe {
        let monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return Err(io::Error::last_os_error());
        }
        let (mut x, mut y) = (96, 96);
        let hr = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y);
        if hr < 0 {
            return Err(io::Error::other(format!("GetDpiForMonitor: HRESULT {hr:#x}")));
        }
        let r = info.rcMonitor;
        Ok(Screen { left: r.left, top: r.top, width: r.right - r.left, height: r.bottom - r.top, scale: x as f32 / 96. })
    }
}

impl Screen {
    pub fn logical_height(self) -> f32 {
        self.height as f32 / self.scale
    }

    fn bounds(self, width: f32, height: f32, right: bool, margin: f32) -> RECT {
        let margin = (margin * self.scale).round() as i32;
        let width = ((width * self.scale).round() as i32).min((self.width - 2 * margin).max(1));
        let height = ((height * self.scale).round() as i32).min((self.height - 2 * margin).max(1));
        let left = self.left + if right { self.width - margin - width } else { margin };
        let top = self.top + margin;
        RECT { left, top, right: left + width, bottom: top + height }
    }
}

/// Restores the prior foreground only while this panel still owns it.
pub struct FocusReturn {
    panel: NonZeroIsize,
    previous: NonZeroIsize,
    previous_pid: u32,
}

impl Drop for FocusReturn {
    fn drop(&mut self) {
        let panel = self.panel.get() as HWND;
        let previous = self.previous.get() as HWND;
        // SAFETY: handles are validated again and are never dereferenced by Rust.
        unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(previous, &mut pid);
            if GetForegroundWindow() == panel && IsWindow(previous) != 0 && pid == self.previous_pid && SetForegroundWindow(previous) == 0 {
                log::debug!("overlay: Windows declined restoring the previous foreground window");
            }
        }
    }
}

/// Configures a hidden window owned by this process, then shows it without
/// activation. Only an explicitly keyboard-opened panel requests foreground.
pub fn show(
    handle: NonZeroIsize,
    screen: Screen,
    size: (f32, f32),
    right: bool,
    margin: f32,
    notice: bool,
    keyboard: bool,
) -> io::Result<Option<FocusReturn>> {
    let hwnd = handle.get() as HWND;
    // SAFETY: style/placement operations use a live HWND of this process only.
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if IsWindow(hwnd) == 0 || pid != GetCurrentProcessId() {
            return Err(io::Error::other("overlay window is not owned by this process"));
        }
        let previous = GetForegroundWindow();
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) & !((WS_CAPTION | WS_THICKFRAME) as isize);
        set_style(hwnd, GWL_STYLE, style)?;
        let mut ex = (GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & !(WS_EX_APPWINDOW as isize)) | WS_EX_TOOLWINDOW as isize;
        if notice || !keyboard {
            ex |= WS_EX_NOACTIVATE as isize;
        }
        if notice {
            ex |= (WS_EX_LAYERED | WS_EX_TRANSPARENT) as isize;
        }
        set_style(hwnd, GWL_EXSTYLE, ex)?;
        if notice && SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA) == 0 {
            return Err(io::Error::last_os_error());
        }
        place(hwnd, screen.bounds(size.0, size.1, right, margin))?;
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        if keyboard && !notice {
            let mut previous_pid = 0;
            GetWindowThreadProcessId(previous, &mut previous_pid);
            if SetForegroundWindow(hwnd) == 0 {
                return Err(io::Error::other("Windows refused keyboard focus for the overlay"));
            }
            return Ok(NonZeroIsize::new(previous as isize).map(|previous| FocusReturn { panel: handle, previous, previous_pid }));
        }
        Ok(None)
    }
}

/// Moving onto a monitor of another DPI sends `WM_DPICHANGED`, and GPUI then
/// resizes the window to Windows' suggestion, scaled from the previous
/// monitor's DPI. Placing it again, now on that monitor, keeps these bounds.
unsafe fn place(hwnd: HWND, bounds: RECT) -> io::Result<()> {
    let wanted = (bounds.left, bounds.top, bounds.right, bounds.bottom);
    let (width, height) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
    // SAFETY: the caller validates the HWND; the rectangle is a local.
    unsafe {
        for _ in 0..3 {
            if SetWindowPos(hwnd, HWND_TOPMOST, bounds.left, bounds.top, width, height, SWP_NOACTIVATE | SWP_FRAMECHANGED) == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut now = RECT::default();
            if GetWindowRect(hwnd, &mut now) == 0 {
                return Err(io::Error::last_os_error());
            }
            if (now.left, now.top, now.right, now.bottom) == wanted {
                return Ok(());
            }
        }
    }
    log::warn!("overlay: Windows kept moving the overlay away from {wanted:?}");
    Ok(())
}

unsafe fn set_style(hwnd: HWND, index: i32, value: isize) -> io::Result<()> {
    // SAFETY: the caller validates the HWND; zero is also a valid previous style.
    unsafe {
        SetLastError(0);
        if SetWindowLongPtrW(hwnd, index, value) == 0 && GetLastError() != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_on_negative_origin_display_at_fractional_dpi() {
        let screen = Screen { left: -2560, top: -200, width: 2560, height: 1440, scale: 1.5 };
        assert_eq!(screen.logical_height(), 960.);
        let r = screen.bounds(440., 912., true, 16.);
        assert_eq!((r.left, r.top, r.right, r.bottom), (-684, -176, -24, 1192));
    }

    #[test]
    fn caps_surfaces_to_the_selected_monitor() {
        let screen = Screen { left: 1920, top: 0, width: 800, height: 600, scale: 1. };
        let r = screen.bounds(1000., 912., false, 16.);
        assert_eq!((r.left, r.top, r.right, r.bottom), (1936, 16, 2704, 584));
    }
}
