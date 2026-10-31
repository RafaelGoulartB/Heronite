//! Opening a file, a folder or a URL with what the desktop picks for it, and
//! running an installer as administrator.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;

/// Opens `target` (a path or a URL) with the desktop's handler, without
/// waiting for it: `xdg-open` on Linux, the shell's "open" on Windows.
pub fn open(target: impl AsRef<OsStr>) -> io::Result<()> {
    imp::open(target.as_ref())
}

/// Why [`run_elevated`] did not run the program.
#[derive(Debug)]
pub enum ElevatedError {
    /// The user said no to Windows' administrator prompt (or nobody was
    /// there to answer it).
    Declined,
    /// It could not be started: missing, not a program, or this system has
    /// no such prompt (Unix).
    Failed(io::Error),
}

impl std::fmt::Display for ElevatedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ElevatedError::Declined => f.write_str("the administrator prompt was declined"),
            ElevatedError::Failed(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ElevatedError {}

/// Runs `program` with `args` as administrator and waits for it: what
/// installers of prerequisites need (Visual C++ runtimes, DirectX, GOG's
/// redistributables, Amazon's `PostInstall`), which fail with error 740
/// ("requires elevation") when started like any program. Windows asks the
/// user first (UAC); a user who says no is [`ElevatedError::Declined`].
/// Gives the program's exit code. Blocking: call it off the UI thread.
///
/// On Unix there is no such prompt: always [`ElevatedError::Failed`].
pub fn run_elevated(program: &Path, args: &[impl AsRef<OsStr>], working_dir: Option<&Path>) -> Result<u32, ElevatedError> {
    let args: Vec<&OsStr> = args.iter().map(AsRef::as_ref).collect();
    imp::run_elevated(program, &quote_args(&args), working_dir)
}

/// `args` as one command line, quoted the way Windows programs split them
/// (`CommandLineToArgvW` and the C runtime): an argument with a space, a
/// tab or a quote, or an empty one, is put in quotes; backslashes before a
/// quote are doubled.
fn quote_args(args: &[&OsStr]) -> OsString {
    let mut line = String::new();
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            line.push(' ');
        }
        let arg = arg.to_string_lossy();
        if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\u{b}', '"']) {
            line.push_str(&arg);
            continue;
        }
        line.push('"');
        let mut backslashes = 0;
        for c in arg.chars() {
            match c {
                '\\' => backslashes += 1,
                '"' => {
                    // Each backslash before a quote, and the quote, escaped.
                    line.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                    line.push('"');
                    backslashes = 0;
                }
                _ => {
                    line.extend(std::iter::repeat_n('\\', backslashes));
                    line.push(c);
                    backslashes = 0;
                }
            }
        }
        // Backslashes before the closing quote are doubled.
        line.extend(std::iter::repeat_n('\\', backslashes * 2));
        line.push('"');
    }
    line.into()
}

#[cfg(unix)]
mod imp {
    use std::ffi::OsStr;
    use std::io;
    use std::path::Path;
    use std::process::{Command, Stdio};

    use super::ElevatedError;

    pub(super) fn open(target: &OsStr) -> io::Result<()> {
        let mut child = Command::new("xdg-open").arg(target).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }

    pub(super) fn run_elevated(_program: &Path, _args: &OsStr, _working_dir: Option<&Path>) -> Result<u32, ElevatedError> {
        Err(ElevatedError::Failed(io::Error::new(io::ErrorKind::Unsupported, "no administrator prompt on this system")))
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::OsStr;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_CANCELLED, WAIT_FAILED};
    use windows_sys::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, ShellExecuteW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    use super::ElevatedError;

    fn wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain([0]).collect()
    }

    pub(super) fn open(target: &OsStr) -> io::Result<()> {
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

    /// A failed `ShellExecuteExW`'s error, the "no" of the prompt apart.
    pub(super) fn error_of(code: u32) -> ElevatedError {
        if code == ERROR_CANCELLED {
            ElevatedError::Declined
        } else {
            ElevatedError::Failed(io::Error::from_raw_os_error(code as i32))
        }
    }

    pub(super) fn run_elevated(program: &Path, args: &OsStr, working_dir: Option<&Path>) -> Result<u32, ElevatedError> {
        let (verb, file, params) = (wide(OsStr::new("runas")), wide(program.as_os_str()), wide(args));
        let dir = working_dir.map(|d| wide(d.as_os_str()));
        // The shell may hand the work to extensions that need COM.
        // SAFETY: plain initialization of this thread; undone below when it succeeded.
        let com = unsafe { CoInitializeEx(std::ptr::null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32) } >= 0;
        // SAFETY: zeroed is a valid value of this plain C struct; the fields set below point to strings that live through the call.
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        // The process handle, no message box on errors, done before returning.
        info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.lpDirectory = dir.as_ref().map_or(std::ptr::null(), |d| d.as_ptr());
        info.nShow = SW_SHOWNORMAL;
        // SAFETY: `info` is filled in as the call expects.
        let ok = unsafe { ShellExecuteExW(&mut info) } != 0;
        // SAFETY: reads this thread's last error, set by ShellExecuteExW.
        let error = unsafe { GetLastError() };
        if com {
            // SAFETY: pairs the successful CoInitializeEx above.
            unsafe { CoUninitialize() };
        }
        if !ok {
            return Err(error_of(error));
        }
        if info.hProcess.is_null() {
            // Handed to a program already running: nothing to wait for.
            return Ok(0);
        }
        let process = info.hProcess;
        // SAFETY: a process handle we own, waited on and closed once.
        unsafe {
            let waited = WaitForSingleObject(process, INFINITE);
            let mut code = 0u32;
            let read = waited != WAIT_FAILED && GetExitCodeProcess(process, &mut code) != 0;
            let error = io::Error::last_os_error();
            CloseHandle(process);
            if read {
                Ok(code)
            } else {
                Err(ElevatedError::Failed(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(args: &[&str]) -> String {
        let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        quote_args(&args).into_string().unwrap()
    }

    #[test]
    fn arguments_are_quoted_the_way_windows_splits_them() {
        assert_eq!(line(&["/install", "/quiet", "/norestart"]), "/install /quiet /norestart");
        assert_eq!(line(&["/D=C:\\Program Files\\Game"]), "\"/D=C:\\Program Files\\Game\"");
        assert_eq!(line(&[""]), "\"\"");
        assert_eq!(line(&["say \"hi\""]), "\"say \\\"hi\\\"\"");
        assert_eq!(line(&["C:\\dir with space\\"]), "\"C:\\dir with space\\\\\"");
        assert_eq!(line(&["a\\\\b", "c\\\"d"]), "a\\\\b \"c\\\\\\\"d\"");
    }

    #[cfg(windows)]
    #[test]
    fn quoted_arguments_split_back_into_the_same_ones() {
        let args = ["/S", "", "two words", "C:\\Program Files\\", "q\"uote", "back\\\\slash\\", "tab\there", "ünï"];
        let quoted = line(&args);
        let back = crate::process::split_command_line_for_tests(&format!("x.exe {quoted}"));
        assert_eq!(back[1..], args);
    }

    #[cfg(windows)]
    #[test]
    fn a_declined_prompt_is_told_apart() {
        assert!(matches!(imp::error_of(1223), ElevatedError::Declined));
        match imp::error_of(2) {
            ElevatedError::Failed(e) => assert_eq!(e.kind(), io::ErrorKind::NotFound),
            other => panic!("{other:?}"),
        }
    }

    // A missing program fails before Windows asks anything.
    #[cfg(windows)]
    #[test]
    fn a_missing_program_is_an_error_without_a_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let err = run_elevated(&dir.path().join("missing-setup.exe"), &["/quiet"], Some(dir.path())).unwrap_err();
        match err {
            ElevatedError::Failed(e) => assert_eq!(e.kind(), io::ErrorKind::NotFound, "{e}"),
            other => panic!("{other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_has_no_prompt() {
        assert!(matches!(run_elevated(Path::new("/bin/true"), &[] as &[&str], None), Err(ElevatedError::Failed(_))));
    }
}
