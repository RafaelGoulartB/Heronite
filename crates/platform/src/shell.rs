//! Opening a file, a folder or a URL with what the desktop picks for it.

use std::ffi::OsStr;
use std::io;

/// Opens `target` (a path or a URL) with the desktop's handler, without
/// waiting for it: `xdg-open` on Linux, the shell's "open" on Windows.
pub fn open(target: impl AsRef<OsStr>) -> io::Result<()> {
    imp::open(target.as_ref())
}

#[cfg(unix)]
mod imp {
    use std::ffi::OsStr;
    use std::io;
    use std::process::{Command, Stdio};

    pub(super) fn open(target: &OsStr) -> io::Result<()> {
        let mut child = Command::new("xdg-open").arg(target).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::OsStr;
    use std::io;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    pub(super) fn open(target: &OsStr) -> io::Result<()> {
        let wide = |s: &OsStr| s.encode_wide().chain([0]).collect::<Vec<u16>>();
        let (verb, file) = (wide(OsStr::new("open")), wide(target));
        // SAFETY: both strings are NUL-terminated and live through the call.
        let r =
            unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
        // Values up to 32 are errors.
        if r as usize > 32 {
            Ok(())
        } else {
            Err(io::Error::other(format!("ShellExecute failed ({})", r as usize)))
        }
    }
}
