//! Processes on Windows: handles from `OpenProcess`, groups as job objects,
//! and the process list from a Toolhelp snapshot.

use std::collections::HashMap;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus};
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JobObjectBasicAccountingInformation, QueryInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
};
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject, CREATE_NEW_PROCESS_GROUP,
    CREATE_NO_WINDOW, DETACHED_PROCESS, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};

use super::Signal;

/// An open handle, closed when dropped.
struct Owned(HANDLE);

// SAFETY: a kernel handle may be used and closed from any thread.
unsafe impl Send for Owned {}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: the handle is ours and closed once.
        unsafe { CloseHandle(self.0) };
    }
}

fn open(pid: u32, access: u32) -> Option<Owned> {
    // SAFETY: OpenProcess has no preconditions; a null handle is a failure.
    let h = unsafe { OpenProcess(access, 0, pid) };
    (!h.is_null()).then_some(Owned(h))
}

pub(super) fn is_alive(pid: u32) -> bool {
    match open(pid, PROCESS_SYNCHRONIZE) {
        // SAFETY: waiting zero milliseconds on a handle we hold.
        Some(h) => unsafe { WaitForSingleObject(h.0, 0) == WAIT_TIMEOUT },
        // SAFETY: reads this thread's last error, set by OpenProcess.
        None => unsafe { GetLastError() == ERROR_ACCESS_DENIED },
    }
}

pub(super) fn signal(pid: u32, _sig: Signal) {
    if let Some(h) = open(pid, PROCESS_TERMINATE) {
        // SAFETY: a handle we hold with the right to terminate.
        unsafe { TerminateProcess(h.0, 1) };
    }
}

/// The job objects of the groups started here, by their first pid. A job
/// lives until its processes are gone, then [`group_alive`] drops it.
fn jobs() -> &'static Mutex<HashMap<u32, Owned>> {
    static JOBS: OnceLock<Mutex<HashMap<u32, Owned>>> = OnceLock::new();
    JOBS.get_or_init(Default::default)
}

pub(super) fn new_group(cmd: &mut Command) -> &mut Command {
    cmd.creation_flags(CREATE_NEW_PROCESS_GROUP)
}

pub(super) fn track_group(child: &Child) {
    // SAFETY: an anonymous job with default security; null is a failure.
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        log::warn!("CreateJobObjectW: {}", std::io::Error::last_os_error());
        return;
    }
    let job = Owned(job);
    // SAFETY: both handles are open; the child's belongs to `child`.
    if unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle() as HANDLE) } == 0 {
        log::warn!("AssignProcessToJobObject({}): {}", child.id(), std::io::Error::last_os_error());
        return;
    }
    jobs().lock().unwrap_or_else(|e| e.into_inner()).insert(child.id(), job);
}

fn active_processes(job: &Owned) -> Option<u32> {
    // SAFETY: zeroed is a valid value of this plain C struct.
    let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: the buffer is the struct the information class asks for.
    let ok = unsafe {
        QueryInformationJobObject(
            job.0,
            JobObjectBasicAccountingInformation,
            (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(info.ActiveProcesses)
}

pub(super) fn group_alive(leader: u32) -> bool {
    let mut jobs = jobs().lock().unwrap_or_else(|e| e.into_inner());
    match jobs.get(&leader).map(active_processes) {
        Some(Some(n)) if n > 0 => true,
        Some(_) => {
            jobs.remove(&leader);
            false
        }
        // Not started here (a game of an earlier run): only the process itself.
        None => is_alive(leader),
    }
}

pub(super) fn signal_group(leader: u32, sig: Signal) {
    let jobs = jobs().lock().unwrap_or_else(|e| e.into_inner());
    match jobs.get(&leader) {
        // SAFETY: a job handle we hold.
        Some(job) => unsafe {
            TerminateJobObject(job.0, 1);
        },
        None => signal(leader, sig),
    }
}

/// Whether `pid` runs in one of the groups started here (the job of a game
/// the app launched, or of a program that game started).
pub fn in_tracked_group(pid: u32) -> bool {
    let Some(process) = open(pid, PROCESS_QUERY_LIMITED_INFORMATION) else { return false };
    let jobs = jobs().lock().unwrap_or_else(|e| e.into_inner());
    jobs.values().any(|job| {
        let mut inside = 0;
        // SAFETY: both handles are open; `inside` receives the answer.
        unsafe { IsProcessInJob(process.0, job.0, &mut inside) != 0 && inside != 0 }
    })
}

pub(super) fn detach(cmd: &mut Command) -> &mut Command {
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
}

pub(super) fn new_helper_group(cmd: &mut Command) -> &mut Command {
    cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
}

pub(super) fn exit_signal(_status: &ExitStatus) -> Option<i32> {
    None
}

pub(super) fn monotonic_ms() -> u64 {
    // SAFETY: no preconditions.
    unsafe { GetTickCount64() }
}

/// A running process, as the Toolhelp snapshot lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: u32,
    pub parent: u32,
    /// The program's file name (`game.exe`).
    pub name: String,
}

/// Every process running now.
pub fn snapshot() -> Vec<ProcessEntry> {
    // SAFETY: a process snapshot; INVALID_HANDLE_VALUE is a failure.
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let snap = Owned(snap);
    // SAFETY: zeroed is a valid value of this plain C struct; dwSize is set below.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut out = Vec::new();
    // SAFETY: the snapshot handle is open and `entry` has its size set.
    let mut more = unsafe { Process32FirstW(snap.0, &mut entry) } != 0;
    while more {
        let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
        out.push(ProcessEntry {
            pid: entry.th32ProcessID,
            parent: entry.th32ParentProcessID,
            name: String::from_utf16_lossy(&entry.szExeFile[..len]),
        });
        // SAFETY: as above.
        more = unsafe { Process32NextW(snap.0, &mut entry) } != 0;
    }
    out
}

/// The full path of the program `pid` runs.
pub fn exe_path(pid: u32) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    let h = open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
    let mut buf = vec![0u16; 32768];
    let mut len = buf.len() as u32;
    // SAFETY: `buf` holds `len` UTF-16 units; the call writes at most that many.
    if unsafe { QueryFullProcessImageNameW(h.0, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) } == 0 {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsString::from_wide(&buf[..len as usize])))
}

/// The arguments `pid` was started with, its program first, split the way
/// Windows programs split them. `None` when the process cannot be read
/// (another user's, or gone).
pub fn command_line(pid: u32) -> Option<Vec<String>> {
    use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessCommandLineInformation};
    use windows_sys::Win32::Foundation::UNICODE_STRING;
    let h = open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
    // u64s: the buffer starts with a UNICODE_STRING, which needs its alignment.
    let mut buf = vec![0u64; 512];
    loop {
        let mut needed = 0u32;
        let size = (buf.len() * 8) as u32;
        // SAFETY: `buf` holds `size` bytes; the call writes at most that many.
        let status = unsafe { NtQueryInformationProcess(h.0, ProcessCommandLineInformation, buf.as_mut_ptr().cast(), size, &mut needed) };
        if status >= 0 {
            break;
        }
        if needed as usize > buf.len() * 8 {
            buf.resize(needed as usize / 8 + 1, 0);
            continue;
        }
        return None;
    }
    // SAFETY: on success the buffer starts with a UNICODE_STRING whose text
    // lies in the same buffer, which outlives this borrow.
    let text = unsafe {
        let s = &*(buf.as_ptr() as *const UNICODE_STRING);
        if s.Buffer.is_null() {
            return Some(Vec::new());
        }
        std::slice::from_raw_parts(s.Buffer, s.Length as usize / 2)
    };
    Some(split_command_line(text))
}

/// Splits a command line like the C runtime does (`CommandLineToArgvW`).
fn split_command_line(line: &[u16]) -> Vec<String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::UI::Shell::CommandLineToArgvW;
    // An empty line would give this program's own path.
    if line.iter().all(|&c| c == u16::from(b' ') || c == u16::from(b'\t')) {
        return Vec::new();
    }
    let text: Vec<u16> = line.iter().copied().chain([0]).collect();
    let mut count = 0i32;
    // SAFETY: `text` is NUL-terminated; the returned array is freed below.
    let argv = unsafe { CommandLineToArgvW(text.as_ptr(), &mut count) };
    if argv.is_null() {
        return Vec::new();
    }
    let args = (0..count.max(0) as usize)
        .map(|i| {
            // SAFETY: `argv` holds `count` NUL-terminated strings.
            unsafe {
                let arg = *argv.add(i);
                let len = (0..).take_while(|&j| *arg.add(j) != 0).count();
                String::from_utf16_lossy(std::slice::from_raw_parts(arg, len))
            }
        })
        .collect();
    // SAFETY: `argv` came from CommandLineToArgvW and is freed once.
    unsafe { LocalFree(argv.cast()) };
    args
}

/// When `pid` started, in milliseconds since 1601 (Windows' file time).
pub fn start_time(pid: u32) -> Option<u64> {
    let h = open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: a handle we hold and four FILETIMEs of ours.
    if unsafe { GetProcessTimes(h.0, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
        return None;
    }
    Some(((created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64) / 10_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_lives_while_any_of_its_processes_does() {
        // `ping` waits about a second per extra echo; the shell ends first.
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start", "/B", "ping", "-n", "30", "127.0.0.1"]);
        new_helper_group(&mut cmd);
        let mut child = cmd.stdout(std::process::Stdio::null()).spawn().unwrap();
        track_group(&child);
        let leader = child.id();
        child.wait().unwrap();
        assert!(group_alive(leader), "ping still runs in the job");
        signal_group(leader, Signal::Term);
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while group_alive(leader) && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(!group_alive(leader));
    }

    #[test]
    fn a_child_s_command_line_comes_back_split() {
        let mut child =
            Command::new("ping").args(["-n", "3", "-w", "100", "127.0.0.1"]).stdout(std::process::Stdio::null()).spawn().unwrap();
        let args = command_line(child.id()).unwrap();
        let _ = child.kill();
        let _ = child.wait();
        assert!(args[0].to_lowercase().contains("ping"), "{args:?}");
        assert_eq!(args[1..], ["-n", "3", "-w", "100", "127.0.0.1"]);
        let wide: Vec<u16> = r#""C:\Program Files\x.exe" -a "b c""#.encode_utf16().collect();
        assert_eq!(split_command_line(&wide), [r"C:\Program Files\x.exe", "-a", "b c"]);
        assert!(split_command_line(&[]).is_empty());
    }

    #[test]
    fn this_process_is_listed_with_its_program() {
        let me = std::process::id();
        assert!(is_alive(me));
        assert!(snapshot().iter().any(|p| p.pid == me));
        let exe = exe_path(me).unwrap();
        assert_eq!(exe, std::env::current_exe().unwrap());
        assert!(start_time(me).unwrap() > 0);
    }
}
