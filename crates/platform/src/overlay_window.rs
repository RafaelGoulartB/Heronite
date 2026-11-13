//! Native placement and focus for GPUI's Windows overlay surfaces.
//!
//! The panel takes the foreground while it is open, however it was opened:
//! XInput has no exclusive grab, but SDL, Unity and Unreal games (most of
//! them) stop reading the controller while their window is in the background,
//! so the overlay's navigation stays out of the game. Notices never activate.

use std::io;
use std::num::NonZeroIsize;

use windows_sys::Win32::Foundation::{GetLastError, SetLastError, HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, HMONITOR, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetActiveWindow, SetFocus};
use windows_sys::Win32::UI::Shell::{SHQueryUserNotificationState, QUERY_USER_NOTIFICATION_STATE, QUNS_RUNNING_D3D_FULL_SCREEN};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsWindow, SetForegroundWindow,
    SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE, GWL_STYLE, HWND_TOPMOST, LWA_ALPHA,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SW_SHOWNOACTIVATE, WS_CAPTION, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TRANSPARENT, WS_THICKFRAME,
};

/// Physical monitor coordinates and its logical-to-physical scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f32,
    /// Physical px the taskbar (or another bar) takes at the bottom: a
    /// surface placed at the bottom sits above it.
    pub bottom_inset: i32,
}

/// The foreground window's monitor, including negative desktop origins.
pub fn foreground_screen() -> io::Result<Screen> {
    // SAFETY: a null foreground window gives the primary monitor.
    screen_of(unsafe { MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY) })
}

/// The monitor holding a desktop point (physical pixels), else the primary.
pub fn screen_at(x: i32, y: i32) -> io::Result<Screen> {
    // SAFETY: no preconditions.
    screen_of(unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTOPRIMARY) })
}

/// Whether a Direct3D app holds the display in exclusive fullscreen, where
/// a topmost window would minimise it or make it switch display modes.
pub fn exclusive_fullscreen() -> bool {
    let mut state: QUERY_USER_NOTIFICATION_STATE = 0;
    // SAFETY: the output is a local.
    let hr = unsafe { SHQueryUserNotificationState(&mut state) };
    hr >= 0 && state == QUNS_RUNNING_D3D_FULL_SCREEN
}

fn screen_of(monitor: HMONITOR) -> io::Result<Screen> {
    // SAFETY: the monitor handle comes from Windows; the output structures are initialized.
    unsafe {
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
        Ok(Screen {
            left: r.left,
            top: r.top,
            width: r.right - r.left,
            height: r.bottom - r.top,
            scale: x as f32 / 96.,
            bottom_inset: (r.bottom - info.rcWork.bottom).max(0),
        })
    }
}

impl Screen {
    pub fn logical_height(self) -> f32 {
        self.height as f32 / self.scale
    }

    fn bounds(self, width: f32, height: f32, right: bool, bottom: bool, margin: f32) -> RECT {
        let margin = (margin * self.scale).round() as i32;
        let width = ((width * self.scale).round() as i32).min((self.width - 2 * margin).max(1));
        let height = ((height * self.scale).round() as i32).min((self.height - 2 * margin).max(1));
        let left = self.left + if right { self.width - margin - width } else { margin };
        let top = self.top + if bottom { (self.height - self.bottom_inset - margin - height).max(margin) } else { margin };
        RECT { left, top, right: left + width, bottom: top + height }
    }
}

/// Restores the prior foreground only while this panel still owns it.
pub struct FocusReturn {
    panel: NonZeroIsize,
    previous: NonZeroIsize,
    previous_pid: u32,
}

impl FocusReturn {
    /// Checks, a moment after the panel opened, that it still has the
    /// foreground, and takes it back from another window of Heronite (the
    /// main window) that got it. Another program's window keeps it: the user
    /// may have switched to it. Call on the UI thread, the panel's.
    pub fn hold(&self) {
        let panel = self.panel.get() as HWND;
        // SAFETY: the handles are only passed to Windows, which checks them.
        unsafe {
            if IsWindow(panel) == 0 {
                return;
            }
            let front = GetForegroundWindow();
            if front == panel {
                return;
            }
            if !ours(front) {
                log::debug!("overlay: {} has the foreground over the panel", describe(front));
                return;
            }
            log::info!("overlay: {} took the foreground back from the panel", describe(front));
            activate(panel);
        }
    }
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
/// activation; a panel (not a notice) then takes the foreground, and the
/// returned [`FocusReturn`] gives it back to the game when the panel closes.
pub fn show(
    handle: NonZeroIsize,
    screen: Screen,
    size: (f32, f32),
    right: bool,
    bottom: bool,
    margin: f32,
    notice: bool,
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
        if notice {
            ex |= (WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT) as isize;
        }
        set_style(hwnd, GWL_EXSTYLE, ex)?;
        if notice && SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA) == 0 {
            return Err(io::Error::last_os_error());
        }
        place(hwnd, screen.bounds(size.0, size.1, right, bottom, margin))?;
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        if notice {
            return Ok(None);
        }
        let mut previous_pid = 0;
        GetWindowThreadProcessId(previous, &mut previous_pid);
        // Refused, the panel still works (the game may also see the
        // controller), a click activates it, and focus still goes back on
        // close if it then owns the foreground.
        activate(hwnd);
        Ok(NonZeroIsize::new(previous as isize).map(|previous| FocusReturn { panel: handle, previous, previous_pid }))
    }
}

/// Brings `hwnd`, a window of this thread, to the foreground, and logs what
/// kept it when that fails. Windows lets only the process that got the last
/// input event take it, and a controller read through XInput is no input
/// event (a keyboard shortcut is). Refused, it is asked again:
/// - Heronite's own window in front (the main window, on this same thread
///   like every GPUI window): activating a window of the thread that has the
///   foreground is always allowed;
/// - another program's window (the game): with this thread's input attached
///   to that window's thread for the call alone, which Windows accepts. No
///   input is sent to the game.
unsafe fn activate(hwnd: HWND) -> bool {
    // SAFETY: the caller passes a window of this thread; the other handles
    // are only passed to Windows, which checks them.
    unsafe {
        if SetForegroundWindow(hwnd) != 0 && GetForegroundWindow() == hwnd {
            return true;
        }
        let front = GetForegroundWindow();
        let thread = GetCurrentThreadId();
        let theirs = GetWindowThreadProcessId(front, std::ptr::null_mut());
        log::info!("overlay: Windows refused the foreground for the panel ({} has it); asking again", describe(front));
        if theirs == thread {
            SetActiveWindow(hwnd);
            SetFocus(hwnd);
        } else if theirs != 0 && AttachThreadInput(thread, theirs, 1) != 0 {
            BringWindowToTop(hwnd);
            SetForegroundWindow(hwnd);
            AttachThreadInput(thread, theirs, 0);
        }
        let front = GetForegroundWindow();
        if front == hwnd {
            log::info!("overlay: the panel has the foreground");
            true
        } else {
            log::info!("overlay: the panel did not get the foreground; {} keeps it", describe(front));
            false
        }
    }
}

/// The process of `hwnd`, 0 for none.
fn process_of(hwnd: HWND) -> u32 {
    let mut pid = 0;
    // SAFETY: Windows checks the handle; the output is a local.
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    pid
}

/// Whether `hwnd` is a window of this process.
fn ours(hwnd: HWND) -> bool {
    let pid = process_of(hwnd);
    // SAFETY: no preconditions.
    pid != 0 && pid == unsafe { GetCurrentProcessId() }
}

/// A window for the logs: whose it is, never its title (it could be private).
fn describe(hwnd: HWND) -> String {
    let pid = process_of(hwnd);
    if pid == 0 {
        "no window".into()
    } else if ours(hwnd) {
        "another Heronite window".into()
    } else {
        format!("a window of process {pid}")
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
        let screen = Screen { left: -2560, top: -200, width: 2560, height: 1440, scale: 1.5, bottom_inset: 0 };
        assert_eq!(screen.logical_height(), 960.);
        let r = screen.bounds(440., 912., true, false, 16.);
        assert_eq!((r.left, r.top, r.right, r.bottom), (-684, -176, -24, 1192));
    }

    #[test]
    fn caps_surfaces_to_the_selected_monitor() {
        let screen = Screen { left: 1920, top: 0, width: 800, height: 600, scale: 1., bottom_inset: 0 };
        let r = screen.bounds(1000., 912., false, false, 16.);
        assert_eq!((r.left, r.top, r.right, r.bottom), (1936, 16, 2704, 584));
    }

    #[test]
    fn a_bottom_surface_sits_above_the_taskbar() {
        let screen = Screen { left: 0, top: 0, width: 1920, height: 1080, scale: 1., bottom_inset: 48 };
        let r = screen.bounds(360., 84., true, true, 24.);
        assert_eq!((r.left, r.top, r.right, r.bottom), (1536, 924, 1896, 1008));
    }
}
