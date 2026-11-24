//! Another program's window brought to the front: a running game's, for the
//! quick launcher's Enter and "Back to game". Only the window manager is
//! asked (which window, then the foreground); nothing is sent to the game
//! and nothing runs in its process.

use std::io;
use std::num::NonZeroIsize;

use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    SetForegroundWindow, ShowWindow, GWL_EXSTYLE, GW_OWNER, SW_RESTORE, WS_EX_TOOLWINDOW,
};

/// Brings the top window of one of `pids` (the highest in the z-order, so
/// the one used last) to the foreground, restoring it when it is minimised.
/// Windows lets the process that has the foreground give it away, so call
/// it while a window of Heronite has it (the quick launcher, the main
/// window, the overlay's panel).
pub fn focus_window_of(pids: &[u32]) -> io::Result<()> {
    let Some(hwnd) = pick(&top_level_windows(), pids) else { return Err(io::Error::other("the game has no window")) };
    let hwnd = hwnd as HWND;
    // SAFETY: a handle EnumWindows gave, only passed to Windows, which checks
    // it (the window may have closed since).
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        if bring_forward(hwnd) {
            Ok(())
        } else {
            Err(io::Error::other("Windows kept the foreground on another window"))
        }
    }
}

/// The foreground window when one of `pids` owns it and it is not
/// minimised. Cheap: no window is listed.
pub fn foreground_of(pids: &[u32]) -> Option<NonZeroIsize> {
    pick_front(foreground(), &[], pids).and_then(usable)
}

/// The window of `pids` being played: the foreground one when it is
/// theirs, else the one [`focus_window_of`] would bring forward. `None`
/// when it is minimised (it then shows on no monitor).
pub fn window_of(pids: &[u32]) -> Option<NonZeroIsize> {
    let front = foreground();
    let hwnd = match pick_front(front, &[], pids) {
        Some(hwnd) => hwnd,
        None => pick_front(front, &top_level_windows(), pids)?,
    };
    usable(hwnd)
}

/// The foreground window and its process (0, 0 for none).
fn foreground() -> (isize, u32) {
    // SAFETY: the handle is only passed to Windows; the output is a local.
    unsafe {
        let front = GetForegroundWindow();
        let mut pid = 0;
        GetWindowThreadProcessId(front, &mut pid);
        (front as isize, pid)
    }
}

/// `hwnd` unless it is null or minimised.
fn usable(hwnd: isize) -> Option<NonZeroIsize> {
    // SAFETY: a handle Windows gave, only passed back to it, which checks it.
    if unsafe { IsIconic(hwnd as HWND) } != 0 {
        return None;
    }
    NonZeroIsize::new(hwnd)
}

/// `front` (the foreground window and its process) when one of `pids` owns
/// it, else the first of theirs in `windows` ([`pick`]).
fn pick_front(front: (isize, u32), windows: &[(isize, u32)], pids: &[u32]) -> Option<isize> {
    let (hwnd, pid) = front;
    if hwnd != 0 && pid != 0 && pids.contains(&pid) {
        return Some(hwnd);
    }
    pick(windows, pids)
}

/// The first window of `windows` (z-order, top first) owned by one of `pids`.
fn pick(windows: &[(isize, u32)], pids: &[u32]) -> Option<isize> {
    windows.iter().find(|(_, pid)| *pid != 0 && pids.contains(pid)).map(|(hwnd, _)| *hwnd)
}

/// The visible top-level windows a user switches to (no tool windows, none
/// owned by another), top first, each with its process.
fn top_level_windows() -> Vec<(isize, u32)> {
    unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> i32 {
        // SAFETY: `data` is the Vec below, alive for the whole EnumWindows;
        // the handle is only passed to Windows.
        unsafe {
            let found = &mut *(data as *mut Vec<(isize, u32)>);
            let tool = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOOLWINDOW as isize != 0;
            if IsWindowVisible(hwnd) != 0 && GetWindow(hwnd, GW_OWNER).is_null() && !tool {
                let mut pid = 0;
                GetWindowThreadProcessId(hwnd, &mut pid);
                found.push((hwnd as isize, pid));
            }
        }
        1
    }
    let mut found: Vec<(isize, u32)> = Vec::new();
    // SAFETY: the callback only touches `found`, which outlives the call.
    unsafe { EnumWindows(Some(each), &mut found as *mut Vec<(isize, u32)> as LPARAM) };
    found
}

/// Gives `hwnd` the foreground. When Windows refuses at first, this
/// thread's input is attached to the thread that has the foreground for the
/// call alone (as `overlay_window` does for the panel), which it accepts.
unsafe fn bring_forward(hwnd: HWND) -> bool {
    // SAFETY: the handles are only passed to Windows, which checks them.
    unsafe {
        if SetForegroundWindow(hwnd) != 0 && GetForegroundWindow() == hwnd {
            return true;
        }
        let thread = GetCurrentThreadId();
        let theirs = GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut());
        let attached = theirs != 0 && theirs != thread && AttachThreadInput(thread, theirs, 1) != 0;
        BringWindowToTop(hwnd);
        SetForegroundWindow(hwnd);
        if attached {
            AttachThreadInput(thread, theirs, 0);
        }
        GetForegroundWindow() == hwnd
    }
}

#[cfg(test)]
mod tests {
    use super::{pick, pick_front};

    #[test]
    fn the_game_in_front_is_the_one_played() {
        let windows = [(1, 10), (3, 30), (4, 20)];
        assert_eq!(pick_front((4, 20), &windows, &[20, 30]), Some(4), "the foreground beats a higher window of the game");
        assert_eq!(pick_front((1, 10), &windows, &[20, 30]), Some(3), "another program in front: the game's top window");
        assert_eq!(pick_front((0, 0), &windows, &[20]), Some(4), "no foreground window");
        assert_eq!(pick_front((1, 10), &[], &[20]), None, "the game in the background, no windows listed");
        assert_eq!(pick_front((5, 0), &windows, &[0]), None, "a window without a process is nobody's");
    }

    #[test]
    fn picks_the_top_window_of_the_game() {
        let windows = [(1, 10), (2, 0), (3, 30), (4, 20), (5, 30)];
        assert_eq!(pick(&windows, &[20, 30]), Some(3), "the highest of the game's windows");
        assert_eq!(pick(&windows, &[40]), None);
        assert_eq!(pick(&windows, &[0]), None, "a window without a process is nobody's");
    }
}
