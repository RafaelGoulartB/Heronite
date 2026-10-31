//! `.lnk` shortcuts (Windows): what they start, with which arguments, from
//! which folder, with which icon and tooltip.
//!
//! The shell's own `ShellLink` object writes and reads them (through COM), so
//! a shortcut holds exactly what Explorer itself would write: the target's
//! item list and link info besides the plain path, and whatever the format
//! gains in later Windows versions. [`encode`] gives the bytes of a shortcut
//! and [`decode`] reads them back, so the caller decides how the file is
//! written (atomically, through a temp file).
//!
//! The arguments of a shortcut are one line that the started program splits
//! again: [`join_args`] quotes each argument by the rules of
//! `CommandLineToArgvW` (the ones Rust's `std::env::args` and the C runtime
//! follow too), and [`split_args`] is the shell's own split, to check them.

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use windows::core::{Interface, HSTRING};
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistStream, IStream, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    COINIT_DISABLE_OLE1DDE, STREAM_SEEK_END, STREAM_SEEK_SET,
};
use windows::Win32::UI::Shell::{IShellLinkW, SHCreateMemStream, ShellLink, SLGP_RAWPATH};

/// What a shortcut starts and how it looks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Link {
    /// The program (or file) it opens.
    pub target: PathBuf,
    /// The command line after the program, already quoted ([`join_args`]).
    pub arguments: String,
    /// The folder the program starts in ("Start in").
    pub working_dir: Option<PathBuf>,
    /// An `.ico`, `.exe` or `.dll` and the index of the icon in it.
    pub icon: Option<(PathBuf, i32)>,
    /// The tooltip ("Comment").
    pub description: String,
}

/// Longest description the shell gives back (`MAX_PATH` with its NUL), in
/// UTF-16 units; longer ones are cut here so a shortcut reads back whole.
const MAX_DESCRIPTION: usize = 259;
/// Room for any path or command line Windows allows.
const BUFFER: usize = 32 * 1024;

/// The bytes of a `.lnk` file for `link`.
pub fn encode(link: &Link) -> io::Result<Vec<u8>> {
    let _com = Com::init();
    // SAFETY: COM is initialized on this thread for the whole block; every
    // string passed lives through its call.
    unsafe {
        let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(err)?;
        shell_link.SetPath(&HSTRING::from(link.target.as_path())).map_err(err)?;
        if !link.arguments.is_empty() {
            shell_link.SetArguments(&HSTRING::from(link.arguments.as_str())).map_err(err)?;
        }
        if let Some(dir) = &link.working_dir {
            shell_link.SetWorkingDirectory(&HSTRING::from(dir.as_path())).map_err(err)?;
        }
        if let Some((file, index)) = &link.icon {
            shell_link.SetIconLocation(&HSTRING::from(file.as_path()), *index).map_err(err)?;
        }
        if !link.description.is_empty() {
            let description: Vec<u16> = link.description.encode_utf16().take(MAX_DESCRIPTION).collect();
            let description = String::from_utf16_lossy(&description);
            shell_link.SetDescription(&HSTRING::from(description)).map_err(err)?;
        }
        let stream = SHCreateMemStream(None).ok_or_else(|| io::Error::other("SHCreateMemStream failed"))?;
        shell_link.cast::<IPersistStream>().map_err(err)?.Save(&stream, true).map_err(err)?;
        read_stream(&stream)
    }
}

/// What the `.lnk` file `bytes` holds.
pub fn decode(bytes: &[u8]) -> io::Result<Link> {
    let _com = Com::init();
    // SAFETY: as in `encode`; the buffers outlive the calls that fill them.
    unsafe {
        let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(err)?;
        let stream = SHCreateMemStream(Some(bytes)).ok_or_else(|| io::Error::other("SHCreateMemStream failed"))?;
        shell_link.cast::<IPersistStream>().map_err(err)?.Load(&stream).map_err(err)?;
        let mut buf = vec![0u16; BUFFER];
        shell_link.GetPath(&mut buf, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32).map_err(err)?;
        let target = PathBuf::from(wide(&buf));
        buf.fill(0);
        shell_link.GetArguments(&mut buf).map_err(err)?;
        let arguments = wide(&buf).to_string_lossy().into_owned();
        buf.fill(0);
        shell_link.GetWorkingDirectory(&mut buf).map_err(err)?;
        let working_dir = Some(PathBuf::from(wide(&buf))).filter(|p| !p.as_os_str().is_empty());
        buf.fill(0);
        let mut index = 0;
        shell_link.GetIconLocation(&mut buf, &mut index).map_err(err)?;
        let icon = Some(PathBuf::from(wide(&buf))).filter(|p| !p.as_os_str().is_empty()).map(|p| (p, index));
        buf.fill(0);
        shell_link.GetDescription(&mut buf).map_err(err)?;
        let description = wide(&buf).to_string_lossy().into_owned();
        Ok(Link { target, arguments, working_dir, icon, description })
    }
}

/// Writes the shortcut `path` (replacing it). Callers that need the file to
/// appear whole at once write [`encode`]'s bytes through a temp file instead.
pub fn write(path: &Path, link: &Link) -> io::Result<()> {
    std::fs::write(path, encode(link)?)
}

/// Reads the shortcut `path`.
pub fn read(path: &Path) -> io::Result<Link> {
    decode(&std::fs::read(path)?)
}

/// Removes the shortcut `path`; `false` when there was none.
pub fn remove(path: &Path) -> io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// One argument quoted so `CommandLineToArgvW` gives it back unchanged:
/// in quotes when it is empty or has a space, tab, newline or quote; inside
/// them a quote becomes `\"` and the backslashes right before a quote (or
/// before the closing one) are doubled. Other backslashes stay as they are.
pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\u{b}', '"']) {
        return arg.to_string();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                // The backslashes before it, doubled, then one for the quote.
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
            }
        }
        if c != '\\' {
            out.push(c);
        }
    }
    // Before the closing quote they would escape it.
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// The arguments as one command line, each quoted by [`quote_arg`].
pub fn join_args<I, S>(args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().map(|a| quote_arg(a.as_ref())).collect::<Vec<_>>().join(" ")
}

/// How the shell splits `line` into arguments (`CommandLineToArgvW`), as the
/// program a shortcut starts sees them after its own name.
pub fn split_args(line: &str) -> Vec<String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::UI::Shell::CommandLineToArgvW;
    // The first word follows other rules (a program path): a plain one in front.
    let full: Vec<u16> = format!("x {line}").encode_utf16().chain([0]).collect();
    let mut count = 0;
    // SAFETY: `full` is NUL-terminated; the returned array holds `count`
    // NUL-terminated strings and is freed once with LocalFree.
    unsafe {
        let argv = CommandLineToArgvW(full.as_ptr(), &mut count);
        if argv.is_null() {
            return Vec::new();
        }
        let args = (1..count.max(0) as usize)
            .map(|i| {
                let p = *argv.add(i);
                let len = (0..).take_while(|&n| *p.add(n) != 0).count();
                String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
            })
            .collect();
        LocalFree(argv.cast());
        args
    }
}

/// COM for the current thread while it lives. A thread already in COM (in
/// another apartment) keeps its own: the shell link object works in both.
struct Com(bool);

impl Com {
    fn init() -> Self {
        // SAFETY: balanced by `CoUninitialize` in `drop` when it succeeded.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if hr.is_err() && hr != RPC_E_CHANGED_MODE {
            log::warn!("CoInitializeEx: {hr:?}");
        }
        Com(hr.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: this thread's successful `CoInitializeEx`.
            unsafe { CoUninitialize() };
        }
    }
}

fn err(e: windows::core::Error) -> io::Error {
    io::Error::other(e)
}

/// The text before the first NUL.
fn wide(buf: &[u16]) -> OsString {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    OsString::from_wide(&buf[..len])
}

/// Everything the stream holds, from its start.
///
/// # Safety
/// COM must be initialized on this thread.
unsafe fn read_stream(stream: &IStream) -> io::Result<Vec<u8>> {
    let mut size = 0u64;
    // SAFETY: the caller's COM; `size` and `out` outlive the calls.
    unsafe {
        stream.Seek(0, STREAM_SEEK_END, Some(&mut size)).map_err(err)?;
        stream.Seek(0, STREAM_SEEK_SET, None).map_err(err)?;
        let mut out = vec![0u8; size as usize];
        let mut read = 0u32;
        stream.Read(out.as_mut_ptr().cast(), out.len() as u32, Some(&mut read)).ok().map_err(err)?;
        out.truncate(read as usize);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(dir: &Path) -> Link {
        Link {
            target: std::env::current_exe().unwrap(),
            arguments: join_args(["--launch", "sideload", "Quail Game \"2\""]),
            working_dir: Some(dir.to_path_buf()),
            icon: Some((dir.join("Cool Game.ico"), 0)),
            description: "Cool Game: the \"sequel\" — ünïcode".to_string(),
        }
    }

    #[test]
    fn a_shortcut_reads_back_what_was_written() {
        let dir = tempfile::tempdir().unwrap();
        let link = sample(dir.path());
        let file = dir.path().join("Cool Game.lnk");
        write(&file, &link).unwrap();
        let bytes = std::fs::read(&file).unwrap();
        // The header of every shell link: its size and CLSID.
        assert_eq!(&bytes[..4], &[0x4c, 0, 0, 0]);
        assert_eq!(&bytes[4..20], &[1, 0x14, 2, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
        assert_eq!(read(&file).unwrap(), link);
        assert_eq!(split_args(&read(&file).unwrap().arguments), ["--launch", "sideload", "Quail Game \"2\""]);
        assert!(remove(&file).unwrap());
        assert!(!file.exists());
        assert!(!remove(&file).unwrap());
    }

    #[test]
    fn empty_parts_stay_empty() {
        let link = Link { target: PathBuf::from(r"C:\Windows\System32\cmd.exe"), ..Default::default() };
        assert_eq!(decode(&encode(&link).unwrap()).unwrap(), link);
    }

    #[test]
    fn long_descriptions_are_cut_to_what_the_shell_keeps() {
        let link = Link { target: std::env::current_exe().unwrap(), description: "x".repeat(5000), ..Default::default() };
        assert_eq!(decode(&encode(&link).unwrap()).unwrap().description, "x".repeat(MAX_DESCRIPTION));
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(decode(b"not a shortcut").is_err());
    }

    #[test]
    fn works_on_a_thread_already_in_the_multithreaded_apartment() {
        std::thread::spawn(|| {
            use windows::Win32::System::Com::COINIT_MULTITHREADED;
            // SAFETY: balanced below.
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
            let dir = tempfile::tempdir().unwrap();
            let link = sample(dir.path());
            assert_eq!(decode(&encode(&link).unwrap()).unwrap(), link);
            // SAFETY: the CoInitializeEx above.
            unsafe { CoUninitialize() };
        })
        .join()
        .unwrap();
    }

    #[test]
    fn quoting_follows_command_line_to_argv() {
        let cases: [(&str, &str); 12] = [
            ("plain", "plain"),
            ("", "\"\""),
            ("two words", "\"two words\""),
            ("tab\there", "\"tab\there\""),
            ("say \"hi\"", r#""say \"hi\"""#),
            (r"C:\Program Files\Heronite\", r#""C:\Program Files\Heronite\\""#),
            (r"C:\no\spaces\", r"C:\no\spaces\"),
            (r#"a\"b"#, r#""a\\\"b""#),
            (r#"a\\"b"#, r#""a\\\\\"b""#),
            (r"back\\slash", r"back\\slash"),
            ("\"", r#""\"""#),
            ("ünï cødé", "\"ünï cødé\""),
        ];
        for (arg, quoted) in cases {
            assert_eq!(quote_arg(arg), quoted, "{arg:?}");
            assert_eq!(split_args(quoted), [arg], "{quoted}");
        }
        let args = ["--launch", "legendary", "", r"C:\a b\", "x\"y", "%PATH%", "a&b|c"];
        assert_eq!(split_args(&join_args(args)), args);
    }
}
