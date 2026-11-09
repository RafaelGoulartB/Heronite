use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus};

use super::Signal;

fn raw(sig: Signal) -> libc::c_int {
    match sig {
        Signal::Term => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    }
}

/// `kill(target, 0)` succeeds, or fails only for lack of permission.
fn exists(target: libc::pid_t) -> bool {
    // SAFETY: signal 0 only checks that the process (group) exists.
    let r = unsafe { libc::kill(target, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

pub(super) fn is_alive(pid: u32) -> bool {
    exists(pid as libc::pid_t)
}

pub(super) fn signal(pid: u32, sig: Signal) {
    // SAFETY: plain kill(2) on a pid we were asked to signal.
    unsafe {
        libc::kill(pid as libc::pid_t, raw(sig));
    }
}

pub(super) fn new_group(cmd: &mut Command) -> &mut Command {
    cmd.process_group(0)
}

pub(super) fn track_group(_child: &Child) {}

pub(super) fn group_alive(leader: u32) -> bool {
    // The kernel keeps a group's id from being given to a new process while
    // the group lives.
    exists(-(leader as libc::pid_t))
}

pub(super) fn signal_group(leader: u32, sig: Signal) {
    // SAFETY: kill(2) on a process group we were asked to signal.
    unsafe {
        libc::kill(-(leader as libc::pid_t), raw(sig));
    }
}

pub(super) fn ask_to_stop(pid: u32, whole_group: bool) {
    if whole_group {
        signal_group(pid, Signal::Term);
    } else {
        signal(pid, Signal::Term);
    }
}

pub(super) fn init_stop_helper() {}

pub(super) fn detach(cmd: &mut Command) -> &mut Command {
    // SAFETY: `setsid` is async-signal-safe and touches no memory of the parent.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        })
    }
}

pub(super) fn new_helper_group(cmd: &mut Command) -> &mut Command {
    cmd.process_group(0)
}

pub(super) fn exit_signal(status: &ExitStatus) -> Option<i32> {
    status.signal()
}

pub(super) fn monotonic_ms() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: clock_gettime writes into the timespec we own.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } != 0 {
        return 0;
    }
    ts.tv_sec as u64 * 1000 + ts.tv_nsec as u64 / 1_000_000
}
